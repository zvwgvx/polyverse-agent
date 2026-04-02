---
title: Configuration
summary: Current configuration files, environment variable groups, and runtime defaults.
order: 40
---

# Configuration

This page is a concise reference for how the runtime is configured today.

## Configuration layers

The main runtime in `apps/agent/src/main.rs` resolves configuration in this order:

1. `.env` via `dotenvy`
2. `config.toml` or the path from `PA_CONFIG`
3. environment overrides for platform tokens, agent identity, and model config
4. `settings.json` for local non-API runtime knobs

Agent profile resolution is separate:

- `config/agent_profile.toml` if present
- otherwise `config/agent_profile.toml.sample`
- then environment overrides handled by the agent profile loader

## Important files

| File | Purpose |
| --- | --- |
| `config/agent_profile.toml.sample` | sample local agent profile, including identity and storage paths |
| `config/prompt_registry.json` | maps logical prompt IDs to files under `prompts/` |
| `config/state_schema.v0.json` | state dimension schema loaded by `state` |
| `config/state_prompt.json` | controls which state domains are injected into dialogue prompts |
| `settings.json` | local non-API overrides such as debug mode, log level, and token limits |
| `config.toml` | optional runtime config file used by `apps/agent` |

## Agent profile defaults

The sample profile defines these default storage paths:

- `data/polyverse-agent/memory.db`
- `data/polyverse-agent/graph`
- `data/polyverse-agent/lancedb`

It also defines identity fields such as `agent_id`, `display_name`, and `graph_self_id`.

## Important environment variable groups

### Dialogue engine

Primary variables:

- `DIALOGUE_ENGINE_API_BASE`
- `DIALOGUE_ENGINE_API_KEY`
- `DIALOGUE_ENGINE_MODEL`
- `DIALOGUE_ENGINE_REASONING`

The runtime also accepts the fallback aliases `OPENAI_API_BASE`, `OPENAI_API_KEY`, `OPENAI_MODEL`, `OPENAI_REASONING`, and the generic `API_BASE`, `API_KEY`, `MODEL`, `REASONING`.

### Affect evaluator

- `AFFECT_EVALUATOR_API_BASE`
- `AFFECT_EVALUATOR_API_KEY`
- `AFFECT_EVALUATOR_MODEL`
- `AFFECT_EVALUATOR_REASONING`

The same OpenAI-style and generic aliases are also accepted here.

### Cockpit

- `COCKPIT_ENABLED`
- `COCKPIT_BIND`
- `COCKPIT_MAX_RECENT_EVENTS`

### MCP

- `MCP_ENABLED`
- `MCP_BIND`
- `MCP_REQUEST_TIMEOUT_MS`
- `MCP_MAX_TOOL_CALLS_PER_TURN`

### State runtime

- `STATE_SCHEMA_PATH`
- `STATE_SYSTEM_ENABLED`
- `STATE_SYSTEM_INTERVAL_MS`

### State prompt injection

- `STATE_PROMPT_CONFIG_PATH`
- `STATE_PROMPT_ENABLED`
- `STATE_PROMPT_PRECISION`
- `STATE_PROMPT_INCLUDE_DERIVED`
- `STATE_PROMPT_DOMAINS`

### Local runtime behavior

- `PA_AGENT_NAME`
- `PA_LOG_LEVEL`
- `DEBUG_MODE`
- `CHAT_MAX_TOKENS`
- `SEMANTIC_MAX_TOKENS`

### Dialogue tool-calling knobs in `settings.json`

Current local settings support these keys:

- `dialogue_tool_calling_enabled`
- `dialogue_tool_max_calls_per_turn`
- `dialogue_tool_timeout_ms`
- `dialogue_tool_max_candidate_users`

## Default local binds

Unless overridden:

- cockpit API binds to `127.0.0.1:4787`
- MCP binds to `127.0.0.1:4790`

## Notes

- `settings.json` does not replace API credentials; it is for local behavior and tuning.
- `scripts/protoc-wrapper.sh` is configured through Cargo and should stay in place when changing protobuf-related build behavior.
- If a behavior change seems prompt-driven rather than code-driven, check `config/prompt_registry.json` and the corresponding files under `prompts/`.
