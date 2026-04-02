use tracing::{debug, warn};

pub fn log_tool_call_start(tool: &str, user_id: &str) {
    debug!(
        kind = "mcp.tool",
        phase = "start",
        tool = tool,
        user = user_id,
        "MCP read tool call started"
    );
}

pub fn log_tool_call_success(tool: &str, user_id: &str) {
    debug!(
        kind = "mcp.tool",
        phase = "success",
        tool = tool,
        user = user_id,
        "MCP read tool call succeeded"
    );
}

pub fn log_tool_call_failure(tool: &str, user_id: &str, error: &str) {
    warn!(
        kind = "mcp.tool",
        phase = "failure",
        tool = tool,
        user = user_id,
        error = error,
        "MCP read tool call failed"
    );
}
