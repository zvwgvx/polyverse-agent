---
title: Dialogue & MCP Tools
summary: The available tool definitions for LLM and MCP consumption.
order: 43
---

# Dialogue & MCP Tools

Both the internal `DialogueEngineWorker` and the external `McpWorker` share the exact same tool registry (`libs/cognitive/src/dialogue_tools.rs`). This guarantees that an external LLM using MCP has the exact same capabilities as the agent's internal brain.

## Namespaces

Tools are strictly partitioned by their mutability guarantees:

1. `ToolNamespace::Read`: Tools that are guaranteed not to alter the state of the agent, graph, or database. They only fetch data. The MCP server only exposes tools in this namespace.
2. `ToolNamespace::Action`: Tools that perform state changes, trigger behaviors, or write data.

## Available Tools

### `social.get_affect_context`
- **Namespace**: `Read`
- **Description**: Fetches the precise emotional and relationship metrics for a specific user.
- **Input Schema**:
  ```json
  {
    "user_id": { "type": "string", "description": "The user identifier to look up" }
  }
  ```
- **Returns**: A JSON payload matching `AffectSocialContext` containing fields like `affinity`, `trust`, `safety`, `tension`, and `context_depth`.

### `social.get_dialogue_summary`
- **Namespace**: `Read`
- **Description**: Fetches a natural language summary of the relationship and coarse state labels.
- **Input Schema**:
  ```json
  {
    "user_id": { "type": "string", "description": "The user identifier to look up" }
  }
  ```
- **Returns**: A JSON payload matching `DialogueSocialSummary` containing a textual `summary`, plus `familiarity`, `trust_state`, and `tension_state` strings.

## Execution Model

When a tool is called, the `DialogueToolRegistry` receives the input JSON, validates it against the schema, performs the execution (usually by calling `query_social_context()`), and returns the `Value`.

If the call originates from the `DialogueEngineWorker`, it runs inline. If it originates from the `McpWorker`, it runs inside the MCP HTTP handler with a strict timeout applied.