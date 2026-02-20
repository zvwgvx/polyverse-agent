use anyhow::{Context, Result};
use reqwest::{Client, header};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct CompressionResult {
    pub fact: String,
    pub importance: f32,
}

pub struct SemanticCompressor {
    client: Client,
    api_base: String,
    api_key: String,
    model: String,
}

impl SemanticCompressor {
    pub fn new() -> Result<Self> {
        let api_base = std::env::var("SLM_API_BASE")
            .unwrap_or_else(|_| "https://openrouter.ai/api/v1".to_string());
        let api_key = std::env::var("SLM_API_KEY")
            .context("SLM_API_KEY missing from environment")?;
        let model = std::env::var("SLM_MODEL")
            .unwrap_or_else(|_| "google/gemini-2.5-flash".to_string());

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
            api_key,
            model,
        })
    }

    /// Takes a chunk of raw conversation text and uses the SLM to extract meaningful facts and rate importance.
    pub async fn compress(&self, raw_transcript: &str) -> Result<Option<CompressionResult>> {
        let system_prompt = "You are the Semantic Filter mechanism of Ryuuko's memory system. \
Your job is to read the following raw chat log of a conversation session and extract EXACTLY ONE core meaningful event/fact. \
If the conversation is pure trivial banter (e.g., just saying hi, checking mic, trivial noise), return nothing. \
If it contains a meaningful event (e.g., User shares a goal, Ryuuko makes a promise, User discusses a project, expressing deep feelings), summarize it into a concise, third-person factual sentence. \
Also provide an importance score from 1.0 to 10.0 (where 1 is trivial, 5 is standard fact, 8+ is a major milestone or emotional bonding moment). \
Return the result strictly as a JSON object: { \"fact\": \"...\", \"importance\": 7.5 }. If nothing is worth saving, return { \"fact\": \"\", \"importance\": 0.0 }. Do NOT add markdown codeblocks around the JSON.";

        let payload = serde_json::json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user", "content": raw_transcript }
            ],
            "response_format": { "type": "json_object" }
        });

        let url = format!("{}/chat/completions", self.api_base);
        
        let res = self.client.post(&url)
            .json(&payload)
            .send()
            .await?;

        let text = res.text().await?;
        let json: serde_json::Value = serde_json::from_str(&text)
            .unwrap_or_else(|_| serde_json::json!({}));

        let content = json.get("choices")
            .and_then(|c| c.as_array())
            .and_then(|c| c.first())
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("");

        if content.is_empty() {
            return Ok(None);
        }

        let parsed: Result<CompressionResult, _> = serde_json::from_str(content).map(|mut val: serde_json::Value| {
            let fact = val.get("fact").and_then(|f| f.as_str()).unwrap_or("").to_string();
            let importance = val.get("importance").and_then(|i| i.as_f64()).unwrap_or(0.0) as f32;
            CompressionResult { fact, importance }
        });

        match parsed {
            Ok(result) if !result.fact.is_empty() && result.importance > 0.0 => Ok(Some(result)),
            _ => Ok(None),
        }
    }
}
