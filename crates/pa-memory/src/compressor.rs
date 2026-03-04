use anyhow::{Context, Result};
use pa_core::prompt_registry::{get_prompt_or, render_prompt_or};
use reqwest::{Client, header};

#[derive(Debug, Clone)]
pub struct CompressionResult {
    pub fact: String,
    pub importance: f32,
}

pub struct SemanticCompressor {
    client: Client,
    api_base: String,
    model: String,
    semantic_max_tokens: u32,
}

impl SemanticCompressor {
    pub fn new() -> Result<Self> {
        let api_base = std::env::var("AFFECT_EVALUATOR_API_BASE")
            .or_else(|_| std::env::var("SYS2_API_BASE"))
            .or_else(|_| std::env::var("OPENAI_API_BASE"))
            .or_else(|_| std::env::var("API_BASE"))
            .unwrap_or_else(|_| "https://openrouter.ai/api/v1".to_string());
            
        let api_key = std::env::var("AFFECT_EVALUATOR_API_KEY")
            .or_else(|_| std::env::var("SYS2_API_KEY"))
            .or_else(|_| std::env::var("OPENAI_API_KEY"))
            .or_else(|_| std::env::var("API_KEY"))
            .context("API_KEY missing from environment")?;
            
        let model = std::env::var("AFFECT_EVALUATOR_MODEL")
            .or_else(|_| std::env::var("SYS2_MODEL"))
            .or_else(|_| std::env::var("OPENAI_MODEL"))
            .or_else(|_| std::env::var("MODEL"))
            .unwrap_or_else(|_| "google/gemini-2.5-flash".to_string());
        
        let semantic_max_tokens = std::env::var("SEMANTIC_MAX_TOKENS")
            .unwrap_or_else(|_| "4096".to_string())
            .parse::<u32>()
            .unwrap_or(4096);

        let mut headers = header::HeaderMap::new();
        let mut auth_val = header::HeaderValue::from_str(&format!("Bearer {}", api_key))?;
        auth_val.set_sensitive(true);
        headers.insert(header::AUTHORIZATION, auth_val);

        let client = Client::builder()
            .default_headers(headers)
            .build()?;

        Ok(Self {
            client,
            api_base,
            model,
            semantic_max_tokens,
        })
    }

    pub async fn compress(&self, base_persona: &str, raw_transcript: &str) -> Result<Option<CompressionResult>> {
        let now = chrono::Utc::now();
        let sg_time = now.with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap());
        let vn_time = now.with_timezone(&chrono::FixedOffset::east_opt(7 * 3600).unwrap());

        let utc_time = now.format("%d/%m/%Y %H:%M:%S").to_string();
        let agent_time = sg_time.format("%d/%m/%Y %H:%M:%S").to_string();
        let user_time = vn_time.format("%d/%m/%Y %H:%M:%S").to_string();
        let time_block = render_prompt_or(
            "memory.compressor.time_block",
            &[
                ("utc_time", utc_time.as_str()),
                ("agent_time", agent_time.as_str()),
                ("user_time", user_time.as_str()),
            ],
            "Current time:\n- UTC: {{utc_time}}\n- Agent: {{agent_time}}\n- User: {{user_time}}\n",
        );
        let diary_cmd = get_prompt_or(
            "memory.compressor.diary_cmd",
            "--- STATE TRANSITION COMMAND ---\nSession ended. Output JSON with one field: \"diary_entry\".",
        );
        let system_prompt_with_diary_cmd =
            format!("{}\n\n{}\n\n{}", base_persona, time_block, diary_cmd);

        let payload = serde_json::json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": system_prompt_with_diary_cmd },
                { "role": "user", "content": raw_transcript }
            ],
            "temperature": 0.7,
            "max_tokens": self.semantic_max_tokens,
            "stream": false,
            "reasoning": {
                "effort": "minimal"
            },
            "provider": {
                "order": ["Google AI Studio"],
                "allow_fallbacks": true
            },
            "response_format": { "type": "json_object" }
        });

        let url = format!("{}/chat/completions", self.api_base);
        
        let mut attempt = 0;
        let max_attempts = 3;

        while attempt < max_attempts {
            attempt += 1;
            
            let res = match self.client.post(&url).json(&payload).send().await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(error = %e, attempt, "Failed to send SLM request, retrying...");
                    tokio::time::sleep(tokio::time::Duration::from_secs(2_u64.pow(attempt as u32))).await;
                    continue;
                }
            };

            let text = match res.text().await {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(error = %e, attempt, "Failed to read SLM response body, retrying...");
                    tokio::time::sleep(tokio::time::Duration::from_secs(2_u64.pow(attempt as u32))).await;
                    continue;
                }
            };

            let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({}));

            let raw_content = json.get("choices")
                .and_then(|c| c.as_array())
                .and_then(|c| c.first())
                .and_then(|c| c.get("message"))
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_str())
                .unwrap_or("");

            if raw_content.is_empty() {
                return Ok(None);
            }

            let cleaned_content = raw_content
                .trim()
                .trim_start_matches("```json")
                .trim_start_matches("```")
                .trim_end_matches("```")
                .trim();

            let parsed: Result<serde_json::Value, _> = serde_json::from_str(cleaned_content);
            match parsed {
                Ok(val) => {
                    let fact = val.get("diary_entry").and_then(|f| f.as_str()).unwrap_or("").to_string();
                    if !fact.is_empty() {
                        return Ok(Some(CompressionResult { fact, importance: 7.0 }));
                    } else {
                        return Ok(None);
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, attempt, "Failed to parse SLM JSON output, retrying...");
                    tokio::time::sleep(tokio::time::Duration::from_secs(2_u64.pow(attempt as u32))).await;
                    continue;
                }
            }
        }

        tracing::error!("Semantic Compressor exhausted all retries.");
        Ok(None)
    }
}
