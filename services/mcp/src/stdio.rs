use std::sync::Arc;

use anyhow::{Context, Result};
use serde_json::{json, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};

use crate::dispatch::{McpDispatcher, ToolCallFailureKind, ToolCallRequest};
use crate::registry::ToolRegistry;

const MCP_PROTOCOL_VERSION: &str = "2025-03-26";
const JSONRPC_VERSION: &str = "2.0";
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INVALID_REQUEST: i64 = -32600;
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
            return Err(error_response(Value::Null, INVALID_REQUEST, "request must be a JSON object"));
        };

        let jsonrpc = object.get("jsonrpc").and_then(|v| v.as_str()).unwrap_or_default();
        if jsonrpc != JSONRPC_VERSION {
            return Err(error_response(Value::Null, INVALID_REQUEST, "jsonrpc must be '2.0'"));
        }

        let method = object
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if method.is_empty() {
            return Err(error_response(
                object.get("id").cloned().unwrap_or(Value::Null),
                INVALID_REQUEST,
                "method is required",
            ));
        }

        Ok(Self {
            id: object.get("id").cloned().unwrap_or(Value::Null),
            method,
            params: object.get("params").cloned().unwrap_or_else(|| json!({})),
        })
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

fn require_initialized(session: &SessionState, id: &Value) -> Option<Value> {
    if session.initialized {
        None
    } else {
        Some(error_response(
            id.clone(),
            NOT_INITIALIZED_ERROR,
            "client must call initialize first",
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
    let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
    Ok((name.to_string(), arguments))
}

fn initialize_result(dispatcher: &McpDispatcher) -> Value {
    json!({
        "protocolVersion": MCP_PROTOCOL_VERSION,
        "capabilities": {
            "tools": {
                "listChanged": false
            }
        },
        "serverInfo": {
            "name": dispatcher.server_name(),
            "version": env!("CARGO_PKG_VERSION")
        }
    })
}

fn tool_list_payload(dispatcher: &McpDispatcher) -> Vec<Value> {
    dispatcher
        .list_registered_tools()
        .into_iter()
        .map(|tool| {
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
        })
        .collect()
}

pub async fn serve_stdio<R, W>(
    input: R,
    mut output: W,
    dispatcher: McpDispatcher,
) -> Result<()>
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

        let request: Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(err) => {
                let payload = error_response(Value::Null, PARSE_ERROR, format!("parse error: {err}"));
                output.write_all(&write_response_line(&payload)).await?;
                output.flush().await?;
                continue;
            }
        };

        let request = match JsonRpcRequest::parse(request) {
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
                if session.initialized {
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
            "notifications/initialized" => None,
            "ping" => {
                if let Some(err) = require_initialized(&session, &request.id) {
                    Some(err)
                } else {
                    Some(success_response(request.id, json!({})))
                }
            }
            "tools/list" => {
                if let Some(err) = require_initialized(&session, &request.id) {
                    Some(err)
                } else {
                    Some(success_response(request.id, json!({ "tools": tool_list_payload(&dispatcher) })))
                }
            }
            "tools/call" => {
                if let Some(err) = require_initialized(&session, &request.id) {
                    Some(err)
                } else {
                    match parse_tools_call(&request.params, &request.id) {
                        Ok((name, arguments)) => match dispatcher.dispatch(ToolCallRequest { name, input: arguments }).await {
                            Ok(result) => Some(success_response(
                                request.id,
                                json!({
                                    "content": [
                                        {
                                            "type": "text",
                                            "text": result.to_string()
                                        }
                                    ],
                                    "structuredContent": result,
                                    "isError": false
                                }),
                            )),
                            Err(err) => {
                                let code = match err.kind {
                                    ToolCallFailureKind::BadRequest => INVALID_PARAMS,
                                    ToolCallFailureKind::Timeout => TOOL_TIMEOUT_ERROR,
                                };
                                Some(error_response(request.id, code, err.message))
                            }
                        },
                        Err(err) => Some(err),
                    }
                }
            }
            "resources/list" => {
                if let Some(err) = require_initialized(&session, &request.id) {
                    Some(err)
                } else {
                    Some(success_response(request.id, json!({ "resources": [] })))
                }
            }
            "prompts/list" => {
                if let Some(err) = require_initialized(&session, &request.id) {
                    Some(err)
                } else {
                    Some(success_response(request.id, json!({ "prompts": [] })))
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

        if let Some(response) = response {
            output.write_all(&write_response_line(&response)).await?;
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
