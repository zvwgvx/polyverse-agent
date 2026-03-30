use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use pa_core::get_agent_profile;
use pa_core::prompt_registry::render_prompt_or;
use pa_memory::{
    episodic::EpisodicStore,
    embedder::MemoryEmbedder,
};
use tokio::sync::Mutex;

#[derive(Clone, Default)]
pub struct SharedContextTiming {
    pub total_ms: u128,
    pub embed_ms: u128,
    pub episodic_search_ms: u128,
    pub chunk_count_ms: u128,
    pub graph_ms: u128,
    pub format_ms: u128,
    pub cache_hit: bool,
}

#[derive(Clone)]
pub struct SharedCognitiveContext {
    pub memory_text: Option<String>,
    pub dialogue_social_text: Option<String>,
    pub time_and_history_text: String,
    pub timing: SharedContextTiming,
}

#[derive(Clone)]
struct CachedSharedContext {
    cached_at: Instant,
    value: SharedCognitiveContext,
}

#[derive(Clone)]
struct CachedChunkCount {
    cached_at: Instant,
    value: usize,
}

static SHARED_CONTEXT_CACHE: OnceLock<
    Mutex<HashMap<String, Arc<Mutex<Option<CachedSharedContext>>>>>,
> = OnceLock::new();
static USER_CHUNK_COUNT_CACHE: OnceLock<Mutex<HashMap<String, CachedChunkCount>>> = OnceLock::new();

const SHARED_CONTEXT_TTL: Duration = Duration::from_secs(30);
const USER_CHUNK_COUNT_TTL: Duration = Duration::from_secs(300);
const MAX_SHARED_CONTEXT_CACHE: usize = 2_048;
const MAX_USER_CHUNK_COUNT_CACHE: usize = 1_024;

fn shared_context_cache() -> &'static Mutex<HashMap<String, Arc<Mutex<Option<CachedSharedContext>>>>> {
    SHARED_CONTEXT_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn user_chunk_count_cache() -> &'static Mutex<HashMap<String, CachedChunkCount>> {
    USER_CHUNK_COUNT_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub async fn build_shared_cognitive_context(
    message_id: &str,
    history: &[(String, String, String)],
    episodic: Option<&Arc<EpisodicStore>>,
    embedder: Option<&Arc<MemoryEmbedder>>,
    current_username: &str,
    new_message: &str,
) -> SharedCognitiveContext {
    let overall_started = Instant::now();
    let slot = {
        let mut cache = shared_context_cache().lock().await;
        if cache.len() > MAX_SHARED_CONTEXT_CACHE {
            cache.clear();
        }
        cache.entry(message_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(None)))
            .clone()
    };

    let mut cached = slot.lock().await;
    if let Some(entry) = cached.as_ref() {
        if entry.cached_at.elapsed() < SHARED_CONTEXT_TTL {
            let mut value = entry.value.clone();
            value.timing.cache_hit = true;
            value.timing.total_ms = overall_started.elapsed().as_millis();
            return value;
        }
    }

    let mut computed = build_shared_cognitive_context_uncached(
        history,
        episodic,
        embedder,
        current_username,
        new_message,
    )
    .await;

    computed.timing.total_ms = overall_started.elapsed().as_millis();

    *cached = Some(CachedSharedContext {
        cached_at: Instant::now(),
        value: computed.clone(),
    });

    computed
}

async fn build_shared_cognitive_context_uncached(
    history: &[(String, String, String)],
    episodic: Option<&Arc<EpisodicStore>>,
    embedder: Option<&Arc<MemoryEmbedder>>,
    current_username: &str,
    new_message: &str,
) -> SharedCognitiveContext {
    let mut timing = SharedContextTiming::default();

    let mut memory_text = None;
    if let (Some(ep), Some(emb)) = (episodic, embedder) {
        let recent_context = history.iter()
            .rev()
            .take(2)
            .map(|(_, _, content)| content.as_str())
            .collect::<Vec<_>>()
            .join(" | ");

        let search_query = if recent_context.is_empty() {
            new_message.to_string()
        } else {
            format!("{} | {}", recent_context, new_message)
        };

        let embed_started = Instant::now();
        if let Ok(query_vec) = emb.embed_single(search_query).await {
            timing.embed_ms = embed_started.elapsed().as_millis();

            let search_started = Instant::now();
            if let Ok(events) = ep.search(&query_vec, 3, 0.5).await {
                timing.episodic_search_ms = search_started.elapsed().as_millis();
                if !events.is_empty() {
                    let mut text = render_prompt_or(
                        "context.memory.header",
                        &[],
                        "### SELF REFLECTION (PAST MEMORY): Use information below only if relevant.\n",
                    );
                    for ev in events {
                        let date_str = chrono::DateTime::from_timestamp(ev.timestamp, 0)
                            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                            .unwrap_or_else(|| "Unknown date".to_string());
                        text.push_str(&render_prompt_or(
                            "context.memory.item",
                            &[("date", date_str.as_str()), ("content", ev.content.as_str())],
                            "- [At {{date}}]: {{content}}\n",
                        ));
                    }
                    memory_text = Some(text);
                }
            } else {
                timing.episodic_search_ms = search_started.elapsed().as_millis();
            }
        } else {
            timing.embed_ms = embed_started.elapsed().as_millis();
        }
    }

    let chunk_count_started = Instant::now();
    let lancedb_count = cached_user_chunk_count(episodic, current_username).await;
    timing.chunk_count_ms = chunk_count_started.elapsed().as_millis();

    let format_started = Instant::now();
    let profile = get_agent_profile();
    let now = chrono::Utc::now();
    let agent_offset = chrono::FixedOffset::east_opt(profile.agent_timezone_offset_hours * 3600)
        .unwrap_or_else(|| chrono::FixedOffset::east_opt(0).expect("valid utc offset"));
    let user_offset = chrono::FixedOffset::east_opt(profile.user_timezone_offset_hours * 3600)
        .unwrap_or_else(|| chrono::FixedOffset::east_opt(0).expect("valid utc offset"));
    let agent_time = now.with_timezone(&agent_offset);
    let user_time = now.with_timezone(&user_offset);
    let mut time_and_history_text = format!(
        "[NOW]: UTC: {} | {}({}): {} | User({}): {}\n",
        now.format("%d/%m/%Y %H:%M:%S"),
        profile.display_name,
        profile.agent_timezone_label,
        agent_time.format("%d/%m/%Y %H:%M:%S"),
        profile.user_timezone_label,
        user_time.format("%d/%m/%Y %H:%M:%S")
    );

    if history.is_empty() {
        if lancedb_count > 0 {
            time_and_history_text.push_str(&render_prompt_or(
                "context.session.first_known",
                &[("username", current_username)],
                "[context: this is the first message of a new session. user ({{username}}) is already known.]",
            ));
        } else {
            time_and_history_text.push_str(&render_prompt_or(
                "context.session.first_unknown",
                &[("username", current_username)],
                "[context: this is the first message from {{username}}. this user is unknown. be cautious.]",
            ));
        }
    } else {
        let mut users = std::collections::HashSet::new();
        for (role, username, _) in history {
            if role == "user" {
                users.insert(username.as_str());
            }
        }
        let users_vec: Vec<&str> = users.into_iter().collect();
        let history_len = history.len().to_string();
        let participants = users_vec.join(", ");
        time_and_history_text.push_str(&render_prompt_or(
            "context.session.has_history",
            &[
                ("history_len", history_len.as_str()),
                ("participants", participants.as_str()),
            ],
            "[context: there were {{history_len}} previous messages. participants: {{participants}}.]",
        ));
    }
    timing.format_ms = format_started.elapsed().as_millis();

    SharedCognitiveContext {
        memory_text,
        dialogue_social_text: None,
        time_and_history_text,
        timing,
    }
}

async fn cached_user_chunk_count(
    episodic: Option<&Arc<EpisodicStore>>,
    current_username: &str,
) -> usize {
    let Some(ep) = episodic else {
        return 0;
    };

    {
        let cache = user_chunk_count_cache().lock().await;
        if let Some(entry) = cache.get(current_username) {
            if entry.cached_at.elapsed() < USER_CHUNK_COUNT_TTL {
                return entry.value;
            }
        }
    }

    let count = ep.count_user_chunks(current_username).await.unwrap_or(0_usize);

    let mut cache = user_chunk_count_cache().lock().await;
    if cache.len() > MAX_USER_CHUNK_COUNT_CACHE {
        cache.clear();
    }
    cache.insert(
        current_username.to_string(),
        CachedChunkCount {
            cached_at: Instant::now(),
            value: count,
        },
    );

    count
}
