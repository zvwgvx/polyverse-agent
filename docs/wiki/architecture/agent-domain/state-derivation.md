---
title: State Derivation
summary: How specialized state workers translate events into numeric deltas.
order: 34
---

# State Derivation

The `StateSystem` (`libs/state/src/lib.rs`) is driven entirely by a family of state derivation workers.

Instead of one monolithic loop trying to understand every event, the runtime decomposes the state update pipeline into distinct, specialized listeners that translate specific `Event`s into `EventDeltaRequest`s.

## The State Workers

The `StateStore` holds the schema and the values. The state derivation workers hold the business logic of *when* to update those values.

### 1. `StateSystemWorker`
**Listens to:** `BiologyEvent`

Translates internal biological ticks (`EnergyChanged`, `SleepStarted`) into numeric state updates for the `system` domain.

- A `SleepStarted` event might set `system.rest_state` to 1.0 (asleep).
- An `EnergyChanged` event with a delta of `-5.0` will result in a `EventDeltaRequest` subtracting 5.0 from `system.energy_level`.

### 2. `StateDriftWorker`
**Listens to:** `SystemEvent` (Tick-based)

This worker runs on a timed loop to enforce the `decay_k` logic defined in `config/state_schema.v0.json`.

For every dimension in the schema, it checks the difference between the current `value` and its `baseline`. It then emits a delta nudging the value closer to the baseline based on the `decay_k` rate. This simulates natural decay over time (e.g., excitement naturally cooling down over a few hours).

### 3. `StateUserWorker`
**Listens to:** `IntentEvent` and `RawEvent`

Translates the context of a user message into the `session_social` domain.

- If the user sends a long message, it might increase `session_social.user_engagement`.
- If the `AffectEvaluatorWorker` classified the message as `Intent::Insult`, the `StateUserWorker` might map that directly to a sharp spike in `session_social.tension`.

## Generating Deltas

When a worker determines a value needs to change, it emits an `EventDeltaRequest` to the `StateStore`.

```rust
pub struct EventDeltaRequest {
    pub dimension_id: String,
    pub delta: f64,
    pub reason: String,
    pub actor: String,
    pub source: String,
}
```

The `StateStore` takes this request, validates it against the schema (`range_min`, `range_max`, `max_delta_per_turn`), updates the floating-point value, and pushes a new `StateDeltaLog` row to the history buffer.

This allows runtime inspection to show precisely *which* worker caused a state change and *why* it happened (e.g., "Energy decay tick", "User insult").