# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Common commands

- Run the main agent: `make agent`
- Run the main agent directly: `cargo run -p pa-agent --bin polyverse-agent`
- Run all Rust tests: `make test` or `cargo test -q`
- Run tests for one crate: `cargo test -p pa-core`
- Run a single Rust test: `cargo test -p pa-core test_event_enum_variants -- --nocapture`
- Check a single crate: `cargo check -p pa-cognitive`
- Build the workspace: `cargo build`
- Build optimized: `cargo build --profile fast-release`
- Run the cockpit web app: `make cockpit`
- Install cockpit dependencies only: `make cockpit-install`
- Typecheck the cockpit web app: `make typecheck`
- Build the cockpit web app: `cd apps/cockpit-web && npm run build`

## Configuration and runtime

- Cargo uses `.cargo/config.toml` to set `PROTOC=scripts/protoc-wrapper.sh`; do not remove that wrapper when changing protobuf-related build behavior.
- The agent loads `.env` first, then `config.toml` or `PA_CONFIG`, and finally environment overrides in `crates/pa-agent/src/main.rs`.
- `config/agent_profile.toml.sample` shows the expected local profile fields. The actual runtime profile is loaded from `config/agent_profile.toml` if present, otherwise the sample, then environment overrides.
- Important checked-in config files:
  - `config/prompt_registry.json` maps logical prompt IDs to files under `prompts/`
  - `config/state_schema.v0.json` defines the cockpit/state dimensions
  - `config/state_prompt.json` controls which state domains are injected into dialogue prompts
- The cockpit web app expects the local cockpit API at `http://127.0.0.1:4787` unless `COCKPIT_API_BASE` is overridden.

## High-level architecture

This repo is a Rust workspace centered on a worker-based agent runtime. `pa-agent` wires the system together; most other crates are subsystems plugged into a shared event bus.

### Runtime shape

- `crates/pa-agent/src/main.rs` is the real composition root.
- `pa-runtime` provides:
  - `Supervisor` to register/start/shutdown workers
  - `EventBus` with an mpsc queue for coordinator input and a broadcast channel for fan-out to workers
  - `Coordinator` as the state machine hub that consumes queued events and rebroadcasts normalized events
- `pa-core` holds the shared contract for the whole system:
  - event types in `crates/pa-core/src/event.rs`
  - worker trait and context in `crates/pa-core/src/worker.rs`
  - agent profile loading and prompt registry helpers

When following control flow, start at `pa-agent/src/main.rs`, then trace worker registration into `Supervisor`, then follow event types through `Coordinator` and the worker implementations.

### Major subsystems

- `pa-sensory`: platform adapters. It converts external platform traffic into `Event::Raw` and consumes `Event::Response` to send replies back out. Current adapters are Discord, Discord selfbot websocket, and Telegram.
- `pa-cognitive`: LLM-facing workers.
  - `DialogueEngineWorker` generates responses from prompt registry content, short-term memory, episodic retrieval, graph context, and optional state snapshots.
  - `AffectEvaluatorWorker` separately scores social/emotional updates and writes them into memory/graph/state.
- `pa-memory`: memory stack with several layers that are easy to confuse if you only read one file:
  - SQLite-backed message persistence (`MemoryStore`)
  - in-process short-term conversational memory (`ShortTermMemory`)
  - semantic compression of completed sessions
  - episodic vector storage in LanceDB
  - relationship/cognitive graph storage in SurrealDB
  `MemoryWorker` is the bridge that listens to broadcast events and keeps these layers in sync.
- `pa-state`: numeric state schema/store plus a family of workers that derive/update state dimensions from events. The store is also exposed to the cockpit and can be injected into the dialogue engine prompt.
- `pa-cockpit-api`: local Axum server exposing observability/debug APIs over worker status, recent events, state, memory, prompts, system metrics, and graph snapshots.
- `apps/cockpit-web`: Next.js frontend that proxies to the local cockpit API.

### Non-obvious design details

- Prompt content is not hardcoded in workers. Prompt IDs are resolved through `config/prompt_registry.json` and loaded from `prompts/**` via `pa-core/src/prompt_registry.rs`. If a behavior change looks “prompty,” inspect the registry and prompt files before editing Rust.
- State injection into model prompts is configurable. `DialogueEngineWorker` reads `config/state_prompt.json` plus env overrides to decide which state domains appear in prompt context.
- `MemoryWorker` batches persistent writes and separately spawns semantic ingestion into episodic memory; message persistence and episodic summarization are intentionally decoupled.
- `AffectEvaluatorWorker` and `DialogueEngineWorker` are independent workers. One generates outward responses; the other updates social/emotional understanding.
- The cockpit API is not just metrics: it can inspect prompts and memory layers, so changes in prompt registry, state schema, episodic memory, or graph code often surface there too.

## Frontend notes

- `apps/cockpit-web` is intentionally minimal: Next.js + React only, with TypeScript typecheck script and a proxy layer under `/api/cockpit/*`.
- When changing cockpit behavior, check both the Axum API shapes in `pa-cockpit-api` and the proxy/frontend expectations in `apps/cockpit-web`.
