# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working in this repository.

## Common commands

- Run the main agent: `make agent`
- Run the main agent directly: `cargo run -p agent --bin polyverse-agent`
- Run the Discord bot service: `make discord`
- Run the Discord selfbot relay: `make discord-selfbot`
- Run the Telegram bot service: `make telegram`
- Run all Rust tests: `make test` or `cargo test -q`
- Run tests for one crate: `cargo test -p kernel`
- Run MCP tests: `cargo test -p mcp`
- Run a single Rust test: `cargo test -p kernel test_event_enum_variants -- --nocapture`
- Check a single crate: `cargo check -p cognitive`
- Build the workspace: `cargo build`
- Build optimized: `cargo build --profile fast-release`
- Run the cockpit web app: `make cockpit`
- Install cockpit dependencies only: `make cockpit-install`
- Typecheck the cockpit web app: `make typecheck`
- Build the cockpit web app: `cd apps/cockpit && npm run build`
- Run the cockpit web app directly: `cd apps/cockpit && npm run dev`

## Workspace overview

This repo is a Rust workspace with these current members:

- `libs/kernel`: shared contracts, event types, worker traits, prompt registry, and agent profile loading
- `libs/runtime`: `Supervisor`, `EventBus`, and `Coordinator`
- `libs/sensory`: shared `SensoryBuffer` and `PlatformAdapter` trait (platform SDK code now lives in platforms/)
- `libs/cognitive`: dialogue engine, affect evaluator, social-query facade, and dialogue tool registry
- `libs/memory`: persistence, short-term memory, episodic memory, semantic compression, and cognitive graph
- `libs/state`: state schema/store and state-derivation workers
- `services/cockpit-api`: local Axum API for observability and prompt/state tooling (embedded worker inside agent)
- `services/mcp`: local read-only MCP transport and tool registry wrapper (embedded worker inside agent)
- `platforms/discord`: standalone Discord bot process (serenity)
- `platforms/discord-selfbot`: standalone Discord selfbot WebSocket relay + Node.js selfbot
- `platforms/telegram`: standalone Telegram bot process (teloxide)
- `apps/agent`: composition root / main binary
- `testing/test-support`: shared test helpers
- `testing/integration-tests`: workspace-level integration tests

## Configuration and runtime

- Cargo uses `.cargo/config.toml` to set `PROTOC=scripts/protoc-wrapper.sh`; do not remove that wrapper when changing protobuf-related build behavior.
- Runtime config is layered in `apps/agent/src/main.rs` like this:
  1. `.env` via `dotenvy`
  2. `config.toml` or the path from `PA_CONFIG`
  3. environment overrides for platform tokens, agent identity, and model config
  4. `settings.json` for local non-API runtime knobs such as `debug_mode`, `log_level`, `chat_max_tokens`, and `semantic_max_tokens`
- `settings.json` is read from the repo root by `agent`; it does not replace API credentials, but it can quietly override local runtime behavior.
- `config/agent_profile.toml.sample` shows the expected profile fields. The actual runtime profile is loaded from `config/agent_profile.toml` if present, otherwise the sample, then environment overrides. `PA_AGENT_PROFILE` can point to an explicit profile file.
- `AgentProfile` also controls default storage paths such as `data/polyverse-agent/memory.db`, `data/polyverse-agent/graph`, and `data/polyverse-agent/lancedb`.
- Important checked-in config files:
  - `config/prompt_registry.json` maps logical prompt IDs to files under `prompts/`
  - `config/state_schema.v0.json` defines the state dimensions loaded by `state`
  - `config/state_prompt.json` controls which state domains are injected into dialogue prompts
- Important runtime env toggles:
  - Cockpit: `COCKPIT_ENABLED`, `COCKPIT_BIND`, `COCKPIT_MAX_RECENT_EVENTS`
  - State runtime: `STATE_SCHEMA_PATH`, `STATE_SYSTEM_ENABLED`, `STATE_SYSTEM_INTERVAL_MS`
  - Dialogue engine: `DIALOGUE_ENGINE_API_BASE`, `DIALOGUE_ENGINE_API_KEY`, `DIALOGUE_ENGINE_MODEL`, `DIALOGUE_ENGINE_REASONING`
  - Affect evaluator: `AFFECT_EVALUATOR_API_BASE`, `AFFECT_EVALUATOR_API_KEY`, `AFFECT_EVALUATOR_MODEL`, `AFFECT_EVALUATOR_REASONING`
  - MCP: `MCP_ENABLED`, `MCP_BIND`, `MCP_REQUEST_TIMEOUT_MS`, `MCP_MAX_TOOL_CALLS_PER_TURN`
  - State prompt overrides: `STATE_PROMPT_CONFIG_PATH`, `STATE_PROMPT_ENABLED`, `STATE_PROMPT_PRECISION`, `STATE_PROMPT_INCLUDE_DERIVED`, `STATE_PROMPT_DOMAINS`
- Default local bindings are `127.0.0.1:4787` for the cockpit API and `127.0.0.1:4790` for the MCP API unless overridden.
- The cockpit web app proxies to `http://127.0.0.1:4787` by default unless `COCKPIT_API_BASE` is overridden.

## High-level architecture

This repo is a worker-based agent runtime. `agent` wires the system together and conditionally registers workers based on env/config.

### Runtime shape

- `apps/agent/src/main.rs` is the real composition root.
- `runtime` provides:
  - `Supervisor` to register/start/shutdown workers
  - `EventBus` with an mpsc queue for coordinator input and a broadcast channel for fan-out to workers
  - `Coordinator` as the state machine hub that consumes queued events and rebroadcasts normalized events
- `kernel` holds the shared contract for the whole system:
  - event types in `libs/kernel/src/event.rs`
  - worker trait and context in `libs/kernel/src/worker.rs`
  - agent profile loading in `libs/kernel/src/agent_profile.rs`
  - prompt registry helpers in `libs/kernel/src/prompt_registry.rs`
- The current runtime wiring in `agent` registers:
  - `MemoryWorker`
  - state workers (`StateDriftWorker`, `StateIntentWorker`, `StateCommandWorker`, `StateUserWorker`, `StateGoalWorker`, `StateEnvironmentWorker`, `StateSystemWorker`)
  - `CockpitWorker`
  - `McpWorker`
  - `DialogueEngineWorker`
  - `AffectEvaluatorWorker`
- Platform bot workers run as **separate processes** (`services/discord`, `services/discord-selfbot`, `services/telegram`) and are launched independently via `make discord`, `make discord-selfbot`, `make telegram`.

When following control flow, start at `agent/src/main.rs`, then trace worker registration into `Supervisor`, then follow event types through `Coordinator` and the worker implementations.

### Major subsystems

- `sensory`: thin shared library containing `SensoryBuffer` (forwards events onto the event bus) and the `PlatformAdapter` trait. Platform SDK code has been extracted to dedicated service crates.
- Platform services: each runs as its own process with its own `Supervisor`+`Coordinator`:
  - `platforms/discord`: serenity Discord bot, reads `DISCORD_BOT_TOKEN`
  - `platforms/discord-selfbot`: WebSocket relay server (port 9000) + Node.js selfbot process, reads `DISCORD_SELFBOT_TOKEN` in `nodejs-selfbot/.env`
  - `platforms/telegram`: teloxide Telegram bot, reads `TELEGRAM_TOKEN`
- `cognitive`: LLM-facing workers and social query logic.
  - `DialogueEngineWorker` builds prompts from prompt registry content, short-term memory, episodic retrieval, graph context, and optional state snapshots.
  - `AffectEvaluatorWorker` separately scores social/emotional updates and writes them into memory/graph/state.
  - `social_context.rs` is now the normalized tree-first query facade for social context, with explicit intents, freshness policy, and fallback metadata.
  - `dialogue_tools.rs` defines the read/action namespace split and the current read-only social tools shared with MCP.
- `memory`: memory stack with several layers that are easy to confuse if you only read one file:
  - SQLite-backed message persistence (`MemoryStore`)
  - in-process short-term conversational memory (`ShortTermMemory`)
  - semantic compression via `SemanticCompressor`
  - episodic vector storage in LanceDB (`EpisodicStore`)
  - relationship/cognitive graph storage in SurrealDB (`CognitiveGraph`)
  - social tree projection/read model (`SocialTreeSnapshot`, `project_social_tree`, `get_or_project_social_tree_snapshot`)
  `MemoryWorker` is the bridge that listens to broadcast events and keeps these layers in sync.
- `state`: numeric state schema/store plus a family of workers that derive/update state dimensions from events. The store is also exposed to the cockpit and can be injected into the dialogue engine prompt.
- `cockpit-api`: local Axum server exposing observability/debug APIs over worker status, recent events, state rows/history/metrics, memory, episodic memory, relationship graph snapshots, system metrics, and prompt documents. It also supports prompt reads/updates and manual state patching.
- `mcp`: local Axum worker exposing read-only MCP-style endpoints:
  - `GET /api/mcp/tools`
  - `POST /api/mcp/tools/call`
  It currently serves `social.get_affect_context` and `social.get_dialogue_summary` by delegating to `cognitive`'s dialogue tool registry.
- `apps/cockpit`: Next.js frontend that proxies to the local cockpit API.

### Non-obvious design details

- Prompt content is not hardcoded in workers. Prompt IDs are resolved through `config/prompt_registry.json` and loaded from `prompts/**` via `kernel/src/prompt_registry.rs`. If a behavior change looks “prompty,” inspect the registry and prompt files before editing Rust.
- `AgentProfile` is more than display metadata: it is the authoritative source for storage paths and graph self identity. If data is being written to an unexpected place, inspect the resolved profile before changing worker code.
- `settings.json` can override local runtime behavior even when `config.toml` is unchanged. If token counts or logging behave unexpectedly, check both env vars and `settings.json`.
- Social retrieval is now intentionally split into write-vs-read layers:
  - Graph = source of truth for writes
  - Social tree snapshot = query/read model
  - `cognitive::social_context` = tree-first query facade with `tree_fresh`, `tree_stale`, `graph_fallback`, and `default_fallback` source metadata
- The MCP registry in `mcp` is intentionally thin. The actual tool definitions and execution logic live in `cognitive::dialogue_tools`, so new read tools usually need changes there first, then transport/tests in `mcp`.
- State injection into model prompts is configurable. `DialogueEngineWorker` reads `config/state_prompt.json` plus env overrides to decide which state domains appear in prompt context.
- `MemoryWorker` batches persistent writes and separately spawns semantic ingestion into episodic memory; message persistence and episodic summarization are intentionally decoupled.
- `AffectEvaluatorWorker` and `DialogueEngineWorker` are independent workers. One generates outward responses; the other updates social/emotional understanding.
- The cockpit API is not just metrics: it can inspect memory layers, relationship graphs, system resources, and prompt files, and it can write prompt updates back through the prompt registry path resolution logic.

## Frontend notes

- `apps/cockpit` is intentionally minimal and currently uses Next.js 15 + React 19.
- The main UI is a single dashboard client component at `apps/cockpit/src/components/dashboard.tsx` with views for overview, metrics, memory, episodic, graph, prompts, and state.
- The frontend proxy layer lives under `apps/cockpit/src/app/api/cockpit/[...path]/route.ts` and forwards requests to the local cockpit API.
- When changing cockpit behavior, check both the Axum API shapes in `cockpit-api` and the proxy/frontend expectations in `apps/cockpit`.
