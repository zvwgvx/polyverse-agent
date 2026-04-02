---
title: Cockpit API
summary: The local Axum observability surface and operations tool.
order: 31
---

# Cockpit API

The `CockpitWorker` exposed by `services/cockpit-api` runs a local HTTP server that provides deep introspection into the running agent. It is intended for developer debugging, operations monitoring, and live manipulation.

It runs on `127.0.0.1:4787` by default. It is the backend that the `apps/cockpit` Next.js app communicates with.

## Observability Capabilities

The API exposes the live state of all worker systems without requiring you to attach a debugger.

- `GET /api/cockpit/status`
  - Returns `CockpitOverview`: uptime, system counters (events handled, bot turns), and the `WorkerStatus` of every registered worker.
- `GET /api/cockpit/events`
  - Returns the rolling log of recent events handled by the `Coordinator` (limited by `COCKPIT_MAX_RECENT_EVENTS`).
- `GET /api/cockpit/system`
  - Returns OS-level telemetry (CPU, memory, disk usage).

## Memory & Graph Inspection

The cockpit provides endpoints to peer into the different layers of the memory stack:
- `GET /api/cockpit/memory`
  - Dumps the recent SQLite `MemoryStore` persistence.
- `GET /api/cockpit/memory/short_term`
  - Returns the live conversational ring-buffer.
- `GET /api/cockpit/memory/episodic`
  - Dumps the LanceDB vector embeddings.
- `GET /api/cockpit/graph/relationships`
  - Compiles and returns a `RelationshipGraphSnapshot` of the SurrealDB cognitive graph, showing how the agent views users.

## Prompt & State Manipulation

The cockpit is not purely read-only. It provides endpoints for hot-swapping behavior and tuning the agent on the fly:

- **State System**:
  - `GET /api/cockpit/state/schema`: Fetch the loaded state schema.
  - `GET /api/cockpit/state/values`: See live emotional and system states.
  - `PATCH /api/cockpit/state/patch`: Send a `ManualPatchRequest` to manually override a state dimension (e.g., forcing `system.energy_level` to zero to test sleep logic).
- **Prompts**:
  - `GET /api/cockpit/prompts`: List all logical prompts mapped in the registry.
  - `GET /api/cockpit/prompts/:id`: Read the raw markdown content of a prompt file.
  - `PATCH /api/cockpit/prompts/:id`: Update a prompt file on disk. The agent will use the new text on the very next turn.