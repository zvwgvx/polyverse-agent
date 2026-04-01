---
title: Architecture
summary: Current runtime shape, main subsystems, and how the repository is composed.
order: 2
---

# Architecture

Polyverse is a worker-based runtime built around a shared event system. The main binary wires together platform adapters, memory and state workers, local service surfaces, and model-facing workers.

## Architectural center of gravity

The most important file for understanding the running system is:

- `apps/agent/src/main.rs`

That file loads config, initializes storage, registers workers, starts the coordinator, and controls shutdown.

## Runtime shape

At a high level, the runtime is built from these pieces:

- `apps/agent` — composition root and binary
- `libs/runtime` — `Supervisor`, `EventBus`, `Coordinator`
- `libs/kernel` — shared event and worker contracts
- `libs/sensory` — platform adapters
- `libs/cognitive` — dialogue engine, affect evaluator, social query helpers, dialogue tools
- `libs/memory` — persistence, short-term memory, episodic memory, graph, social tree projection
- `libs/state` — state schema/store and state-derived workers
- `services/cockpit-api` — local cockpit API
- `services/mcp` — local read-only MCP surface

## Main subsystem roles

### Runtime and contracts

- `runtime` manages worker lifecycle and event coordination.
- `kernel` defines the event types and worker interfaces used everywhere else.

### Sensory layer

`sensory` converts platform traffic into `Event::Raw` and consumes response events to send replies back out.

### Cognitive layer

`cognitive` contains the model-facing workers and social query logic.

Current notable pieces include:

- `DialogueEngineWorker`
- `AffectEvaluatorWorker`
- `social_context.rs`
- `dialogue_tools.rs`

### Memory layer

`memory` is not one storage system. It is a stack of several layers used for different purposes, including persistent chat history, short-term memory, episodic memory, and the cognitive graph.

### State layer

`state` maintains numeric state dimensions and exposes workers that update state from events.

### Local service surfaces

- cockpit exposes observability and prompt/state tooling
- MCP exposes a read-only HTTP interface for tool-style access

## Notes on current scope

Some older documents in `docs/` describe broader planned systems. This wiki focuses on the current runtime and only treats future designs as background when they are clearly labeled as such.

## Read next

- [Runtime Flow](./runtime-flow.md) for startup and event movement through the runtime
- [Memory Stack](./memory-stack.md) for the current memory layers and social query model
