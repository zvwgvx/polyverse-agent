---
title: Repository Map
summary: Top-level repository areas and the role of each major package group.
order: 11
---

# Repository Map

The repository is organized by role rather than by language alone. Rust packages live in the Cargo workspace, while the Next.js apps sit alongside them under `apps/`.

## Top-level areas

| Path | Role | Current contents |
| --- | --- | --- |
| `apps/` | runnable applications and user-facing entrypoints | `agent`, `wiki` |
| `libs/` | reusable internal libraries | `kernel`, `runtime`, `sensory`, `cognitive`, `memory`, `state` |
| `services/` | local service and transport boundaries | `mcp` |
| `testing/` | shared test support and integration coverage | `test-support`, `integration-tests` |
| `docs/` | repository documentation | legacy notes plus the public `docs/wiki` subtree |
| `config/` | checked-in config inputs | agent profile sample, prompt registry, state schema, state prompt config |
| `prompts/` | prompt text files | prompt documents resolved through the registry |
| `data/` | local runtime data | default storage area used by the agent profile |
| `scripts/` | repository helper scripts | includes the protobuf wrapper used by Cargo builds |

## Current app surfaces

### `apps/agent`

The main Rust binary and composition root. It loads config, initializes storage, registers workers, starts the coordinator, and runs until shutdown.

### `apps/wiki`

A filesystem-driven Next.js app that renders `docs/wiki` as the public technical wiki.

## Current library packages

- `libs/kernel` — shared contracts, events, worker traits, agent profile loading, prompt registry helpers
- `libs/runtime` — `Supervisor`, `EventBus`, `Coordinator`
- `libs/sensory` — Discord and Telegram platform adapters
- `libs/cognitive` — dialogue engine, affect evaluator, social query facade, dialogue tool registry
- `libs/memory` — persistence, short-term memory, episodic memory, cognitive graph, social tree projection
- `libs/state` — state schema/store and state-derived workers

## Current service packages

- `services/mcp` — local read-only MCP transport and registry wrapper

## Current test packages

- `testing/test-support` — helpers and fixtures shared by tests
- `testing/integration-tests` — cross-package integration and runtime smoke coverage

## Notes on older docs

Some older documents still reference a historical `crates/` layout. Prefer the live `apps/`, `libs/`, `services/`, and `testing/` paths when reading the current codebase.
