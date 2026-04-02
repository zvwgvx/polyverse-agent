---
title: State Schema Dimensions
summary: The domains and dimensions tracked by the state system.
order: 44
---

# State Schema Dimensions

The state dimensions defined in `config/state_schema.v0.json` form the vocabulary for the agent's internal and environmental awareness.

These dimensions drift over time (`decay_k`) toward their `baseline` and are constrained between `range_min` and `range_max`.

## Domains

### 1. `emotion`
Tracks the internal feelings of the agent.
- `emotion.arousal`: (0.0 to 1.0) The intensity or energy of the agent's current feeling.
- `emotion.valence`: (-1.0 to 1.0) The positivity or negativity of the agent's mood.

### 2. `system`
Tracks biological-analog state and physical resources.
- `system.energy_level`: (0.0 to 100.0) The overall power reserve. Depletes with activity.
- `system.rest_state`: (0.0 to 1.0) Whether the agent is currently "asleep" or inactive.
- `system.focus`: (0.0 to 1.0) The current capacity to process complex intents vs. simple ones.

### 3. `session_social`
Tracks the immediate, ephemeral social dynamics of the current chat session. (Note: Deep relationships go to the Graph, not State).
- `session_social.tension`: (0.0 to 1.0) How stressful or combative the current conversation is.
- `session_social.user_engagement`: (0.0 to 1.0) How actively users are talking.

### 4. `environment`
Tracks the context of the platform the agent is inhabiting.
- `environment.noise_level`: (0.0 to 1.0) How many irrelevant or background messages are flying past.
- `environment.activity_spike`: (0.0 to 1.0) A short-lived metric that jumps when many messages arrive at once.

## Update Modes

Each dimension has an `update_mode` that dictates how workers modify it:
- `Delta`: Workers supply a `+` or `-` value, which is added to the current value (clamped by `max_delta_per_turn`).
- `Absolute`: Workers provide an exact new value.

## Access

To see the exact mathematical tuning for each dimension (the specific baselines and decay rates), inspect `config/state_schema.v0.json` directly. The Cockpit API (`GET /api/cockpit/state/schema`) also returns this data live.