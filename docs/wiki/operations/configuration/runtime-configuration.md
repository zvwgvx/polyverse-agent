---
title: Runtime Configuration
summary: How the main runtime loads configuration, applies overrides, and exposes local service settings.
order: 31
---

# Runtime Configuration

The main runtime is configured in `apps/agent/src/main.rs`. This page explains the current layering model and the most important runtime knobs.

## Configuration layering

At startup, the runtime resolves configuration in this order:

1. `.env` via `dotenvy`
2. `config.toml` or the file pointed to by `PA_CONFIG`
3. environment overrides for platform tokens, agent identity, and model config
4. `settings.json` for local non-API behavior such as logging and token limits

This means the live runtime can differ from `config.toml` if environment variables or `settings.json` override specific values.

## `settings.json`

`settings.json` is read from the repository root and currently supports keys such as:

- `debug_mode`
- `log_level`
- `chat_max_tokens`
- `semantic_max_tokens`
- `dialogue_tool_calling_enabled`
- `dialogue_tool_max_calls_per_turn`
- `dialogue_tool_timeout_ms`
- `dialogue_tool_max_candidate_users`

These are local runtime knobs. They do not replace model API credentials.

## Agent profile

The agent profile is resolved through the agent profile loader and controls identity plus storage paths.

Important fields in the sample profile include:

- `agent_id`
- `display_name`
- `graph_self_id`
- `memory_db_path`
- `graph_db_path`
- `episodic_db_path`

If the runtime writes data to an unexpected place, the agent profile is one of the first files to check.

## Runtime service defaults

### Cockpit

Default behavior:

- `COCKPIT_ENABLED=true` unless overridden
- default bind: `127.0.0.1:4787`
- default recent event cap: `300`

Cockpit depends on the state schema loading successfully.

### MCP

MCP is opt-in and is loaded through `load_mcp_config()`.

Defaults when enabled:

- `MCP_BIND=127.0.0.1:4790`
- `MCP_REQUEST_TIMEOUT_MS=2000`
- `MCP_MAX_TOOL_CALLS_PER_TURN=4`

The runtime also clamps unsafe minimums for timeout and tool-call count.

### State runtime

Important state settings:

- `STATE_SCHEMA_PATH` defaults to `config/state_schema.v0.json`
- `STATE_SYSTEM_ENABLED=true` unless overridden
- `STATE_SYSTEM_INTERVAL_MS=1000` unless overridden

## Model configuration

### Dialogue engine

The dialogue engine reads config file values first and then accepts environment overrides for:

- API base
- API key
- model
- reasoning mode

The runtime also resolves `CHAT_MAX_TOKENS` and dialogue tool-calling settings before constructing `DialogueEngineConfig`.

### Affect evaluator

The affect evaluator is registered only when its API base, key, and model are available through environment resolution.

## State prompt configuration

`config/state_prompt.json` currently enables snapshot injection with:

- `precision: 3`
- `include_derived: true`
- domains: `session_social`, `emotion`, `system`, `environment`

The state legend is currently disabled.

## Prompt registry

Prompt documents are not hardcoded into workers. They are resolved through `config/prompt_registry.json`, which maps logical prompt IDs to files under `prompts/`.

## Notes

- Cockpit and MCP are local/internal surfaces by default, not public deployment targets.
- The runtime creates missing storage directories for memory, graph, and episodic data based on the resolved agent profile.
- If a runtime claim is unclear, `apps/agent/src/main.rs` is the best source of truth.
