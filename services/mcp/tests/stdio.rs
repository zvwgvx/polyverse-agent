use std::sync::Arc;

use anyhow::Result;
use memory::graph::CognitiveGraph;
use mcp::{registry::ToolRegistry, stdio::serve_stdio, McpDispatcher};
use serde_json::json;
use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt, BufReader};

async fn in_memory_graph() -> CognitiveGraph {
    CognitiveGraph::new("memory")
        .await
        .expect("in-memory graph should initialize")
}

#[tokio::test]
async fn stdio_server_supports_initialize_ping_list_resources_prompts_and_call() -> Result<()> {
    let graph = in_memory_graph().await;
    let dispatcher = McpDispatcher::new(Arc::new(ToolRegistry::default()), graph, 2000);

    let (client_side, server_side) = duplex(8192);
    let (server_read, server_write) = tokio::io::split(server_side);
    let server = tokio::spawn(async move { serve_stdio(BufReader::new(server_read), server_write, dispatcher).await });

    let (mut client_read, mut client_write) = tokio::io::split(client_side);
    client_write
        .write_all(format!("{}\n", json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}})).as_bytes())
        .await?;
    client_write
        .write_all(format!("{}\n", json!({"jsonrpc":"2.0","method":"notifications/initialized","params":{}})).as_bytes())
        .await?;
    client_write
        .write_all(format!("{}\n", json!({"jsonrpc":"2.0","id":2,"method":"ping","params":{}})).as_bytes())
        .await?;
    client_write
        .write_all(format!("{}\n", json!({"jsonrpc":"2.0","id":3,"method":"tools/list","params":{}})).as_bytes())
        .await?;
    client_write
        .write_all(format!("{}\n", json!({"jsonrpc":"2.0","id":4,"method":"resources/list","params":{}})).as_bytes())
        .await?;
    client_write
        .write_all(format!("{}\n", json!({"jsonrpc":"2.0","id":5,"method":"prompts/list","params":{}})).as_bytes())
        .await?;
    client_write
        .write_all(format!("{}\n", json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"social.get_affect_context","arguments":{"user_id":"alice","memory_hint":0.2}}})).as_bytes())
        .await?;
    client_write.shutdown().await?;

    let mut buf = Vec::new();
    client_read.read_to_end(&mut buf).await?;
    server.await??;

    let output = String::from_utf8(buf)?;
    let lines: Vec<&str> = output.lines().filter(|line| !line.trim().is_empty()).collect();
    assert_eq!(lines.len(), 6);

    let init: serde_json::Value = serde_json::from_str(lines[0])?;
    assert_eq!(init.get("id").and_then(|v| v.as_i64()), Some(1));
    assert_eq!(
        init.get("result")
            .and_then(|v| v.get("protocolVersion"))
            .and_then(|v| v.as_str()),
        Some("2025-03-26")
    );

    let ping: serde_json::Value = serde_json::from_str(lines[1])?;
    assert_eq!(ping.get("id").and_then(|v| v.as_i64()), Some(2));
    assert_eq!(ping.get("result").and_then(|v| v.as_object()).map(|v| v.len()), Some(0));

    let list: serde_json::Value = serde_json::from_str(lines[2])?;
    assert_eq!(list.get("id").and_then(|v| v.as_i64()), Some(3));
    assert_eq!(
        list.get("result")
            .and_then(|v| v.get("tools"))
            .and_then(|v| v.as_array())
            .map(|arr| arr.len()),
        Some(2)
    );
    assert!(list
        .get("result")
        .and_then(|v| v.get("tools"))
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|tool| tool.get("inputSchema"))
        .is_some());

    let resources: serde_json::Value = serde_json::from_str(lines[3])?;
    assert_eq!(resources.get("id").and_then(|v| v.as_i64()), Some(4));
    assert_eq!(
        resources.get("result")
            .and_then(|v| v.get("resources"))
            .and_then(|v| v.as_array())
            .map(|arr| arr.len()),
        Some(0)
    );

    let prompts: serde_json::Value = serde_json::from_str(lines[4])?;
    assert_eq!(prompts.get("id").and_then(|v| v.as_i64()), Some(5));
    assert_eq!(
        prompts.get("result")
            .and_then(|v| v.get("prompts"))
            .and_then(|v| v.as_array())
            .map(|arr| arr.len()),
        Some(0)
    );

    let call: serde_json::Value = serde_json::from_str(lines[5])?;
    assert_eq!(call.get("id").and_then(|v| v.as_i64()), Some(6));
    assert_eq!(
        call.get("result")
            .and_then(|v| v.get("isError"))
            .and_then(|v| v.as_bool()),
        Some(false)
    );
    assert!(call
        .get("result")
        .and_then(|v| v.get("structuredContent"))
        .and_then(|v| v.get("metrics"))
        .is_some());

    Ok(())
}

#[tokio::test]
async fn stdio_server_returns_json_rpc_errors_for_bad_calls() -> Result<()> {
    let graph = in_memory_graph().await;
    let dispatcher = McpDispatcher::new(Arc::new(ToolRegistry::default()), graph, 2000);

    let (client_side, server_side) = duplex(4096);
    let (server_read, server_write) = tokio::io::split(server_side);
    let server = tokio::spawn(async move { serve_stdio(BufReader::new(server_read), server_write, dispatcher).await });

    let (mut client_read, mut client_write) = tokio::io::split(client_side);
    client_write
        .write_all(format!("{}\n", json!({"jsonrpc":"2.0","id":8,"method":"initialize","params":{}})).as_bytes())
        .await?;
    client_write
        .write_all(format!("{}\n", json!({"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"social.get_dialogue_summary","arguments":{}}})).as_bytes())
        .await?;
    client_write.shutdown().await?;

    let mut buf = Vec::new();
    client_read.read_to_end(&mut buf).await?;
    server.await??;

    let output = String::from_utf8(buf)?;
    let lines: Vec<&str> = output.lines().filter(|line| !line.trim().is_empty()).collect();
    let line = lines
        .iter()
        .find(|line| line.contains("\"id\":9"))
        .copied()
        .expect("tools/call response line should exist");
    let payload: serde_json::Value = serde_json::from_str(line)?;
    assert_eq!(payload.get("id").and_then(|v| v.as_i64()), Some(9));
    assert_eq!(payload.get("error").and_then(|v| v.get("code")).and_then(|v| v.as_i64()), Some(-32602));
    assert!(payload
        .get("error")
        .and_then(|v| v.get("message"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .contains("missing field `user_id`"));

    Ok(())
}
