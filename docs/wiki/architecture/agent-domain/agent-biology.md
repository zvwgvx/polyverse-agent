---
title: Agent Biology
summary: How the Coordinator ticks the internal sleep, energy, and mood cycles.
order: 22
---

# Agent Biology

Agents in Polyverse are not purely reactive state machines. They have an internal "biological" clock managed entirely by the `Coordinator` (`libs/runtime/src/coordinator.rs`).

## The Biology Event

The biological system is built around the `BiologyEventKind` enum:

- `EnergyChanged { delta: f32, reason: String }`
- `MoodChanged { new_mood: String, trigger: String }`
- `SleepStarted`
- `SleepEnded`

## The Coordinator Tick Loop

The `Coordinator` runs an infinite loop that constantly consumes external events (from the `mpsc` queue) and broadcasts them out to workers.

In addition to routing chat messages, the `Coordinator` has a periodic internal "tick". This tick represents time passing in the agent's world.

### Energy Decay

Over time, or in response to high-activity environments, the `Coordinator` generates and emits `Event::Biology(BiologyEventKind::EnergyChanged)`. 

This is broadcast over the `EventBus` to the rest of the system. Cognitive workers and State workers hear this event and can react accordingly:
- `StateSystemWorker` receives it and deducts points from the numeric `system.energy_level` dimension.
- If energy drops too low, the agent may emit `SleepStarted` and refuse to engage in non-critical dialogue until `SleepEnded` fires.

### Mood Shifts

Similarly, repeated negative sentiment events (classified by the `AffectEvaluatorWorker` and caught by the `Coordinator`) can trigger a `MoodChanged` event. This allows the state system to slowly shift the `emotion.valence` and `emotion.arousal` baselines over time.

## Why is Biology Centralized?

By centralizing the biological clock in the `Coordinator` rather than isolating it in a specific worker:
1. **Consistency**: Time and energy decay happen exactly once, in deterministic order alongside chat messages.
2. **Global Awareness**: Every single worker (Memory, Dialogue, and others) sees the agent's energy shift immediately via the broadcast channel, so they can halt processing or adjust their prompt behavior synchronously.