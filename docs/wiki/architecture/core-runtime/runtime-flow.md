---
title: Runtime Flow
summary: Startup path, worker registration, and event movement through the current runtime.
order: 20
---

# Runtime Flow

The runtime starts in `apps/agent/src/main.rs`. That file is the composition root for the running system.

## Startup sequence

At a high level, startup follows this order:

1. load config from `.env`, config files, environment overrides, and `settings.json`
2. load the agent profile and resolve storage paths
3. initialize logging
4. create a `Supervisor`
5. load state schema if available
6. initialize episodic storage, embedder, compressor, and cognitive graph
7. conditionally register workers based on config and available credentials
8. create the `Coordinator`
9. start all workers through the supervisor
10. run until shutdown, then stop workers and abort the coordinator task

## Core runtime primitives

### `Supervisor`

`Supervisor` owns worker registration, startup, and shutdown.

It is responsible for:

- storing registered workers
- starting all workers with a shared context
- shutting them down cleanly

### `EventBus`

`EventBus` provides two channels:

- an internal queue used to feed the coordinator
- a broadcast channel used to fan out normalized events to workers

### `Coordinator`

`Coordinator` consumes events from the event queue and rebroadcasts them to workers. It also owns the shared biology state that is updated by biology events.

## Worker registration in the current runtime

Depending on config, the main binary can register:

- sensory workers for Discord bot, Discord selfbot websocket, and Telegram
- `MemoryWorker`
- state workers such as drift, intent, command, user, goal, environment, and system workers
- `McpWorker`
- `DialogueEngineWorker`
- `AffectEvaluatorWorker`

No single worker is mandatory. The runtime can run with a subset depending on available config.

## Event flow

A typical message-driven path looks like this:

1. a sensory adapter receives platform input
2. the adapter emits `Event::Raw`
3. the event enters the coordinator through the event queue
4. the coordinator rebroadcasts it through the event bus
5. memory, state, dialogue, and other interested workers consume it
6. the dialogue worker may emit `Event::Response` and `Event::BotTurnCompletion`
7. sensory workers consume response events to send output back to the platform

## Evidence from tests

The current runtime behavior is backed by integration tests such as:

- `testing/integration-tests/tests/runtime_wiring.rs`
- `testing/integration-tests/tests/runtime_supervisor_mcp.rs`
- `testing/integration-tests/tests/dialogue_worker_smoke.rs`

These tests cover coordinator broadcasting, supervisor lifecycle behavior, live MCP startup, and dialogue worker response flow.

## Shutdown

On `Ctrl+C`, the main process:

1. signals shutdown through the supervisor
2. shuts down registered workers
3. aborts the coordinator task
4. exits after logging that the agent has stopped

## Practical reading path

If you want to follow this flow in code, start at:

1. `apps/agent/src/main.rs`
2. `libs/runtime/src/supervisor.rs`
3. `libs/runtime/src/event_bus.rs`
4. `libs/runtime/src/coordinator.rs`
5. the worker implementations in `libs/` and `services/`
