#[derive(Debug, Clone)]
pub struct McpConfig {
    pub enabled: bool,
    pub bind_addr: String,
    pub request_timeout_ms: u64,
    pub max_tool_calls_per_turn: usize,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind_addr: "127.0.0.1:4790".to_string(),
            request_timeout_ms: 2_000,
            max_tool_calls_per_turn: 4,
        }
    }
}

impl McpConfig {
    pub fn from_env() -> Self {
        let enabled = std::env::var("MCP_ENABLED")
            .ok()
            .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
            .unwrap_or(false);

        let bind_addr = std::env::var("MCP_BIND")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "127.0.0.1:4790".to_string());

        let request_timeout_ms = std::env::var("MCP_REQUEST_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(|v| v.max(100))
            .unwrap_or(2_000);

        let max_tool_calls_per_turn = std::env::var("MCP_MAX_TOOL_CALLS_PER_TURN")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .map(|v| v.max(1))
            .unwrap_or(4);

        Self {
            enabled,
            bind_addr,
            request_timeout_ms,
            max_tool_calls_per_turn,
        }
    }
}
