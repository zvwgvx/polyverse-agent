use std::sync::Arc;

use anyhow::{Context, Result};
use serde_json::{json, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};

use crate::dispatch::{McpDispatcher, ToolCallFailure, ToolCallFailureKind, ToolCallRequest};
use crate::registry::ToolRegistry;

const MCP_PROTOCOL_VERSION: &str = "2025-03-26";
const JSONRPC_VERSION: &str = "2.0";

const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const PARSE_ERROR: i64 = -32700;

const TOOL_TIMEOUT_ERROR: i64 = -32000;
const NOT_INITIALIZED_ERROR: i64 = -32002;
const ALREADY_INITIALIZED_ERROR: i64 = -32003;

#[derive(Default)]
struct SessionState {
    initialized: bool,
}

#[derive(Debug)]
struct JsonRpcRequest {
    id: Value,
    method: String,
    params: Value,
}

impl JsonRpcRequest {
    fn parse(value: Value) -> Result<Self, Value> {
        let Some(object) = value.as_object() else {
            return Err(error_response(
                Value::Null,
                INVALID_REQUEST,
                "request must be a JSON object",
            ));
        };

        let jsonrpc = object
            .get("jsonrpc")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if jsonrpc != JSONRPC_VERSION {
            return Err(error_response(
                Value::Null,
                INVALID_REQUEST,
                "jsonrpc must be '2.0'",
            ));
        }

        let id = object.get("id").cloned().unwrap_or(Value::Null);
        let method = object
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if method.is_empty() {
            return Err(error_response(id, INVALID_REQUEST, "method is required"));
        }

        let params = object.get("params").cloned().unwrap_or_else(|| json!({}));
        Ok(Self { id, method, params })
    }
}

fn success_response(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": JSONRPC_VERSION,
        "id": id,
        "result": result,
    })
}

fn error_response(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": JSONRPC_VERSION,
        "id": id,
        "error": {
            "code": code,
            "message": message.into(),
        }
    })
}

fn write_response_line(response: &Value) -> Vec<u8> {
    let mut bytes = response.to_string().into_bytes();
    bytes.push(b'\n');
    bytes
}

fn ensure_object_params(params: &Value, id: &Value, method: &str) -> Result<(), Value> {
    if params.is_object() {
        Ok(())
    } else {
        Err(error_response(
            id.clone(),
            INVALID_PARAMS,
            format!("{method} params must be a JSON object"),
        ))
    }
}

fn parse_tools_call(params: &Value, id: &Value) -> Result<(String, Value), Value> {
    let Some(name) = params.get("name").and_then(|v| v.as_str()) else {
        return Err(error_response(
            id.clone(),
            INVALID_PARAMS,
            "tools/call requires a string name",
        ));
    };

    let name = name.trim();
    if name.is_empty() {
        return Err(error_response(
            id.clone(),
            INVALID_PARAMS,
            "tools/call requires a non-empty name",
        ));
    }

    let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
    if !arguments.is_object() {
        return Err(error_response(
            id.clone(),
            INVALID_PARAMS,
            "tools/call arguments must be a JSON object",
        ));
    }

    Ok((name.to_string(), arguments))
}

fn parse_tools_get(params: &Value, id: &Value) -> Result<String, Value> {
    let Some(name) = params.get("name").and_then(|v| v.as_str()) else {
        return Err(error_response(
            id.clone(),
            INVALID_PARAMS,
            "tools/get requires a string name",
        ));
    };

    let name = name.trim();
    if name.is_empty() {
        return Err(error_response(
            id.clone(),
            INVALID_PARAMS,
            "tools/get requires a non-empty name",
        ));
    }

    Ok(name.to_string())
}

fn parse_logging_set_level(params: &Value, id: &Value) -> Result<(), Value> {
    let Some(level) = params.get("level").and_then(|v| v.as_str()) else {
        return Err(error_response(
            id.clone(),
            INVALID_PARAMS,
            "logging/setLevel requires a string level",
        ));
    };

    let valid = ["trace", "debug", "info", "warn", "error"];
    if valid.contains(&level) {
        Ok(())
    } else {
        Err(error_response(
            id.clone(),
            INVALID_PARAMS,
            "logging/setLevel level must be one of: trace, debug, info, warn, error",
        ))
    }
}

fn tools_capability(dispatcher: &McpDispatcher) -> Value {
    if !dispatcher.supports_tools() {
        return Value::Null;
    }

    json!({
        "listChanged": false,
        "get": true,
        "supportsExecution": dispatcher.supports_execution_tools(),
    })
}

fn initialize_result(dispatcher: &McpDispatcher) -> Value {
    json!({
        "protocolVersion": MCP_PROTOCOL_VERSION,
        "capabilities": {
            "tools": tools_capability(dispatcher),
            "resources": Value::Null,
            "prompts": Value::Null,
            "logging": Value::Null,
            "streaming": Value::Null
        },
        "serverInfo": {
            "name": dispatcher.server_name(),
            "version": env!("CARGO_PKG_VERSION")
        }
    })
}

fn tool_payload(tool: crate::provider::RegisteredTool) -> Value {
    json!({
        "name": tool.descriptor.name,
        "title": tool.descriptor.name,
        "description": tool.description,
        "inputSchema": tool.input_schema,
        "annotations": {
            "readOnlyHint": tool.descriptor.read_only,
            "openWorldHint": false
        }
    })
}

fn tool_list_payload(dispatcher: &McpDispatcher) -> Vec<Value> {
    dispatcher
        .list_registered_tools()
        .into_iter()
        .map(tool_payload)
        .collect()
}

fn tools_get_payload(dispatcher: &McpDispatcher, name: &str) -> Option<Value> {
    dispatcher
        .get_registered_tool(name)
        .map(|tool| json!({ "tool": tool_payload(tool) }))
}

fn map_tool_call_error(err: ToolCallFailure) -> (i64, String) {
    match err.kind {
        ToolCallFailureKind::Timeout => (TOOL_TIMEOUT_ERROR, err.message),
        ToolCallFailureKind::BadRequest => {
            if let Some(tool_name) = err.message.strip_prefix("unknown MCP tool:") {
                let name = tool_name.trim();
                return (METHOD_NOT_FOUND, format!("unknown tool: {name}"));
            }
            (INVALID_PARAMS, err.message)
        }
    }
}

async fn handle_tools_call(dispatcher: &McpDispatcher, request_id: Value, params: Value) -> Value {
    let (name, arguments) = match parse_tools_call(&params, &request_id) {
        Ok(parsed) => parsed,
        Err(payload) => return payload,
    };

    match dispatcher
        .dispatch(ToolCallRequest {
            name,
            input: arguments,
        })
        .await
    {
        Ok(result) => success_response(
            request_id,
            json!({
                "content": [{ "type": "text", "text": result.to_string() }],
                "structuredContent": result,
                "isError": false
            }),
        ),
        Err(err) => {
            let (code, message) = map_tool_call_error(err);
            error_response(request_id, code, message)
        }
    }
}

fn handle_tools_get(dispatcher: &McpDispatcher, request_id: Value, params: Value) -> Value {
    let name = match parse_tools_get(&params, &request_id) {
        Ok(name) => name,
        Err(payload) => return payload,
    };

    match tools_get_payload(dispatcher, &name) {
        Some(result) => success_response(request_id, result),
        None => error_response(request_id, METHOD_NOT_FOUND, format!("unknown tool: {name}")),
    }
}

async fn handle_initialized_method(
    dispatcher: &McpDispatcher,
    request_id: Value,
    method: &str,
    params: Value,
) -> Value {
    match method {
        "ping" => match ensure_object_params(&params, &request_id, "ping") {
            Ok(()) => success_response(request_id, json!({})),
            Err(err) => err,
        },
        "tools/list" => match ensure_object_params(&params, &request_id, "tools/list") {
            Ok(()) => success_response(request_id, json!({ "tools": tool_list_payload(dispatcher) })),
            Err(err) => err,
        },
        "tools/get" => match ensure_object_params(&params, &request_id, "tools/get") {
            Ok(()) => handle_tools_get(dispatcher, request_id, params),
            Err(err) => err,
        },
        "resources/list" => match ensure_object_params(&params, &request_id, "resources/list") {
            Ok(()) => success_response(request_id, json!({ "resources": [] })),
            Err(err) => err,
        },
        "prompts/list" => match ensure_object_params(&params, &request_id, "prompts/list") {
            Ok(()) => success_response(request_id, json!({ "prompts": [] })),
            Err(err) => err,
        },
        "logging/setLevel" => match parse_logging_set_level(&params, &request_id) {
            Ok(()) => success_response(request_id, json!({})),
            Err(err) => err,
        },
        "tools/call" => handle_tools_call(dispatcher, request_id, params).await,
        _ => error_response(request_id, METHOD_NOT_FOUND, format!("method not found: {method}")),
    }
}

pub async fn serve_stdio<R, W>(input: R, mut output: W, dispatcher: McpDispatcher) -> Result<()>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut lines = input.lines();
    let mut session = SessionState::default();

    while let Some(line) = lines.next_line().await? {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let request_value: Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(err) => {
                let payload = error_response(Value::Null, PARSE_ERROR, format!("parse error: {err}"));
                output.write_all(&write_response_line(&payload)).await?;
                output.flush().await?;
                continue;
            }
        };

        let request = match JsonRpcRequest::parse(request_value) {
            Ok(req) => req,
            Err(payload) => {
                output.write_all(&write_response_line(&payload)).await?;
                output.flush().await?;
                continue;
            }
        };

        let is_notification = request.id.is_null();

        let response = match request.method.as_str() {
            "initialize" => {
                if let Err(err) = ensure_object_params(&request.params, &request.id, "initialize") {
                    Some(err)
                } else if session.initialized {
                    Some(error_response(
                        request.id,
                        ALREADY_INITIALIZED_ERROR,
                        "initialize may only be called once",
                    ))
                } else {
                    session.initialized = true;
                    Some(success_response(request.id, initialize_result(&dispatcher)))
                }
            }
            "notifications/initialized" => {
                let _ = ensure_object_params(&request.params, &request.id, "notifications/initialized");
                None
            }
            "ping"
            | "tools/list"
            | "tools/get"
            | "resources/list"
            | "prompts/list"
            | "tools/call"
            | "logging/setLevel" => {
                if !session.initialized {
                    Some(error_response(
                        request.id,
                        NOT_INITIALIZED_ERROR,
                        format!("{} requires initialize first", request.method),
                    ))
                } else {
                    Some(
                        handle_initialized_method(
                            &dispatcher,
                            request.id,
                            request.method.as_str(),
                            request.params,
                        )
                        .await,
                    )
                }
            }
            _ => Some(error_response(
                request.id,
                METHOD_NOT_FOUND,
                format!("method not found: {}", request.method),
            )),
        };

        if is_notification {
            continue;
        }

        if let Some(payload) = response {
            output.write_all(&write_response_line(&payload)).await?;
            output.flush().await?;
        }
    }

    Ok(())
}

pub async fn serve_process_stdio(dispatcher: McpDispatcher) -> Result<()> {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let reader = BufReader::new(stdin);
    serve_stdio(reader, stdout, dispatcher)
        .await
        .context("stdio MCP server failed")
}

pub fn build_stdio_dispatcher(
    registry: Arc<ToolRegistry>,
    graph: memory::graph::CognitiveGraph,
    request_timeout_ms: u64,
) -> McpDispatcher {
    McpDispatcher::new(registry, graph, request_timeout_ms)
}
