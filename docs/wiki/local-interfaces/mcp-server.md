---
title: MCP Server
summary: The local Model Context Protocol endpoint for AI integrations.
order: 32
---

# MCP Server

The `McpWorker` implemented in `services/mcp` runs a local HTTP server that exposes a Model Context Protocol (MCP) compatible API. It binds to `127.0.0.1:4790` by default.

This server allows external AI tools (like Claude Code) or even the agent's own internal models to query its internal state via standardized tool calls.

## Endpoints

The worker exposes a simplified, stateless HTTP representation of MCP:

### `GET /api/mcp/tools`
Returns the list of all registered tools, mapping standard `ToolNamespace` tools into MCP JSON schema shapes.

### `POST /api/mcp/tools/call`
Executes a tool. Expects a JSON body with `name` and `input`.
Returns `{"ok": true, "result": {...}}` on success or `{"ok": false, "error": "..."}` on failure.

## Execution and Timeout

Unlike normal Axum routes, the MCP tool execution endpoint wraps every call in a `tokio::time::timeout`. 
The timeout is controlled by the `MCP_REQUEST_TIMEOUT_MS` environment variable (default: `15000` ms). 

If a tool (such as a complex graph query) takes too long, the endpoint gracefully catches it and returns a standard timeout error JSON shape, preventing rogue external calls from hanging the worker thread indefinitely.

## Read-only enforcement

Currently, the MCP server is strictly read-only. It delegates execution to the `DialogueToolRegistry`, which currently only registers tools under `ToolNamespace::Read`.

The MCP server is specifically designed to allow external observers to read the state of the graph or memory (e.g., `social.get_affect_context`) without allowing them to mutate it. Any future "action" capabilities will require explicit `ToolNamespace::Action` enablement.