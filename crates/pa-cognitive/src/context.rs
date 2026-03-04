use std::sync::Arc;
use pa_core::prompt_registry::render_prompt_or;
use pa_memory::{
    episodic::EpisodicStore,
    embedder::MemoryEmbedder,
    graph::CognitiveGraph,
};

pub struct SharedCognitiveContext {
    pub memory_text: Option<String>,
    pub social_text: String,
    pub time_and_history_text: String,
}

pub async fn build_shared_cognitive_context(
    history: &[(String, String, String)],
    episodic: Option<&Arc<EpisodicStore>>,
    embedder: Option<&Arc<MemoryEmbedder>>,
    graph: &CognitiveGraph,
    current_username: &str,
    new_message: &str,
) -> SharedCognitiveContext {
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

        if let Ok(query_vec) = emb.embed_single(search_query).await {
            if let Ok(events) = ep.search(&query_vec, 3, 0.5).await {
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
            }
        }
    }

    let lancedb_count = if let Some(ep) = episodic {
        ep.count_user_chunks(current_username).await.unwrap_or(0_usize)
    } else {
        0_usize
    };
    let memory_hint = (lancedb_count as f32 / 200.0).min(0.15);

    let social_text;

    if let Ok((attitudes, illusion)) = graph.get_social_context(current_username).await {
        let graph_depth = (attitudes.affinity.abs() + attitudes.attachment.abs() 
            + attitudes.trust.abs() + attitudes.safety.abs()) / 4.0;
        let context_depth = (graph_depth + memory_hint).min(1.0);
        let affinity = format!("{:.6}", attitudes.affinity);
        let attachment = format!("{:.6}", attitudes.attachment);
        let trust = format!("{:.6}", attitudes.trust);
        let safety = format!("{:.6}", attitudes.safety);
        let tension = format!("{:.6}", attitudes.tension);
        let depth = format!("{:.6}", context_depth);
        let ill_affinity = format!("{:.6}", illusion.affinity);
        let ill_attachment = format!("{:.6}", illusion.attachment);
        let ill_trust = format!("{:.6}", illusion.trust);
        let ill_safety = format!("{:.6}", illusion.safety);
        let ill_tension = format!("{:.6}", illusion.tension);

        social_text = render_prompt_or(
            "context.social.known",
            &[
                ("username", current_username),
                ("affinity", affinity.as_str()),
                ("attachment", attachment.as_str()),
                ("trust", trust.as_str()),
                ("safety", safety.as_str()),
                ("tension", tension.as_str()),
                ("context_depth", depth.as_str()),
                ("ill_affinity", ill_affinity.as_str()),
                ("ill_attachment", ill_attachment.as_str()),
                ("ill_trust", ill_trust.as_str()),
                ("ill_safety", ill_safety.as_str()),
                ("ill_tension", ill_tension.as_str()),
            ],
            "### EMOTIONAL AND RELATION STATE WITH {{username}}:\nAffinity: {{affinity}}\nAttachment: {{attachment}}\nTrust: {{trust}}\nSafety: {{safety}}\nTension: {{tension}}\nContext Depth: {{context_depth}}\nAssumed perception -> Affinity: {{ill_affinity}}, Attachment: {{ill_attachment}}, Trust: {{ill_trust}}, Safety: {{ill_safety}}, Tension: {{ill_tension}}\n",
        );
    } else {
        let depth = format!("{:.6}", memory_hint);
        social_text = render_prompt_or(
            "context.social.default",
            &[("username", current_username), ("context_depth", depth.as_str())],
            "### EMOTIONAL AND RELATION STATE WITH {{username}}:\nAffinity: 0.000000\nAttachment: 0.000000\nTrust: 0.000000\nSafety: 0.000000\nTension: 0.000000\nContext Depth: {{context_depth}}\nAssumed perception -> Affinity: 0.000000, Attachment: 0.000000, Trust: 0.000000, Safety: 0.000000, Tension: 0.000000\n",
        );
    }

    let now = chrono::Utc::now();
    let sg_time = now.with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap());
    let vn_time = now.with_timezone(&chrono::FixedOffset::east_opt(7 * 3600).unwrap());
    let mut time_and_history_text = format!(
        "[NOW]: UTC: {} | Ryuuko(GMT+8): {} | User(GMT+7): {}\n",
        now.format("%d/%m/%Y %H:%M:%S"),
        sg_time.format("%d/%m/%Y %H:%M:%S"),
        vn_time.format("%d/%m/%Y %H:%M:%S")
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

    SharedCognitiveContext {
        memory_text,
        social_text,
        time_and_history_text,
    }
}
