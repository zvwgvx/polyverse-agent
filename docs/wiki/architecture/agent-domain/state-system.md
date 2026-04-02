---
title: State System
summary: The numeric state tracking system and derivation worker pipeline.
order: 26
---

# State System

While the memory graph stores deep structural relationships, the State System (`libs/state`) manages continuous, numeric dimensions representing the agent's internal conditions, environmental awareness, and conversational flow.

## 1. The Schema

The universe of possible state variables is defined by a JSON schema, not hardcoded Rust structs. By default, `config/state_schema.v0.json` is loaded on startup.

Each dimension in the schema has properties like:
- `id`: Unique identifier (e.g., `emotion.arousal`, `system.energy_level`)
- `domain`: High-level category
- `range_min` / `range_max`: Boundaries
- `baseline`: Natural resting state
- `decay_k`: Rate at which it drifts back to baseline

## 2. The `StateStore`

The `StateStore` holds the live values of all dimensions in memory. It tracks:
- **Current Values**: The current floating-point value.
- **Delta History**: A rolling log of the last 2,000 changes, who made them, and why.
- **Metrics**: Hit counts showing which sources modify state most frequently.

The store enforces the rules defined in the schema: if a worker tries to add `+5.0` to a value with a `range_max` of `1.0`, the store clamps it safely.

## 3. State Workers

Instead of one monolithic loop updating state, Polyverse uses a family of specialized workers that listen to the `EventBus` and emit state deltas.

Examples include:
- `StateSystemWorker`: Listens to `BiologyEvent`s and translates them into numeric updates for `system.energy_level` or `system.rest_state`.
- `StateDriftWorker`: A tick-based worker that periodically nudges all values closer to their configured `baseline` according to their `decay_k`.
- `StateUserWorker`: Analyzes incoming user messages to update `session_social.user_engagement`.

## 4. Prompt Injection

State isn't just for metrics; it directly influences the LLM. 

The `config/state_prompt.json` file configures which state domains (e.g., `emotion`, `system`) are injected into the Dialogue Engine's system prompt context. This allows the LLM to implicitly "feel" its energy level or the current tension of the room without hardcoding those concepts into the core logic.

## 5. Cockpit Exposure

The Cockpit API has deep hooks into the state system:
- `GET /api/cockpit/state/schema`: Read the live schema.
- `GET /api/cockpit/state/values`: Read the current values.
- `GET /api/cockpit/state/history`: Read the delta log.
- `PATCH /api/cockpit/state/patch`: Manually overwrite a dimension for testing/debugging.