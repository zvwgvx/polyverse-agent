#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpTransport {
    Http,
    Stdio,
}

impl Default for McpTransport {
    fn default() -> Self {
        Self::Http
    }
}

impl McpTransport {
    fn from_env_value(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "http" => Some(Self::Http),
            "stdio" => Some(Self::Stdio),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct McpConfig {
    pub enabled: bool,
    pub transport: McpTransport,
    pub bind_addr: String,
    pub request_timeout_ms: u64,
    pub max_tool_calls_per_turn: usize,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            transport: McpTransport::Http,
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

        let transport = std::env::var("MCP_TRANSPORT")
            .ok()
            .as_deref()
            .and_then(McpTransport::from_env_value)
            .unwrap_or_default();

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
            transport,
            bind_addr,
            request_timeout_ms,
            max_tool_calls_per_turn,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{McpConfig, McpTransport};
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn clear_env() {
        unsafe {
            std::env::remove_var("MCP_ENABLED");
            std::env::remove_var("MCP_TRANSPORT");
            std::env::remove_var("MCP_BIND");
            std::env::remove_var("MCP_REQUEST_TIMEOUT_MS");
            std::env::remove_var("MCP_MAX_TOOL_CALLS_PER_TURN");
        }
    }

    #[test]
    fn from_env_supports_stdio_transport() {
        let _guard = env_lock().lock().unwrap_or_else(|p| p.into_inner());
        clear_env();
        unsafe {
            std::env::set_var("MCP_TRANSPORT", "stdio");
        }
        let config = McpConfig::from_env();
        assert_eq!(config.transport, McpTransport::Stdio);
        clear_env();
    }

    #[test]
    fn from_env_defaults_to_http_and_keeps_bind_backward_compatibility() {
        let _guard = env_lock().lock().unwrap_or_else(|p| p.into_inner());
        clear_env();
        unsafe {
            std::env::set_var("MCP_BIND", "0.0.0.0:9999");
        }
        let config = McpConfig::from_env();
        assert_eq!(config.transport, McpTransport::Http);
        assert_eq!(config.bind_addr, "0.0.0.0:9999");
        clear_env();
    }
}
