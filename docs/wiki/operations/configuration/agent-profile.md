---
title: Agent Profile
summary: How agent identity and storage paths are loaded.
order: 34
---

# Agent Profile

The `AgentProfile` is the foundational configuration struct that defines *who* the agent is and *where* it stores its data. It's loaded before the runtime even starts.

## Loading the Profile

The composition root (`apps/agent/src/main.rs`) initializes the `AgentProfile` via `get_agent_profile()`.

The loading sequence is:
1. It looks for a TOML file at the path specified by the `PA_AGENT_PROFILE` environment variable.
2. If the variable is not set or the file is missing, it falls back to `config/agent_profile.toml`.
3. If that file is also missing, it loads `config/agent_profile.toml.sample`.
4. It then applies environment variable overrides (e.g., `PA_AGENT_NAME` overrides the TOML's `display_name`).

## Identity Fields

The profile defines the agent's core identity variables used in logging, prompt templating, and graph storage:

- `agent_id`: The internal system identifier (e.g., `agent`).
- `display_name`: The human-readable name injected into the system prompt (e.g., `Agent`).
- `graph_self_id`: The exact SurrealDB node ID representing the agent itself (e.g., `person:agent`).

## Storage Paths

Crucially, the profile determines the paths where the `apps/agent` binary will spin up the local databases. By default:

- `memory_db_path`: `data/polyverse-agent/memory.db`
- `graph_db_path`: `data/polyverse-agent/graph`
- `episodic_db_path`: `data/polyverse-agent/lancedb`

If you are running multiple instances of the agent on the same machine (e.g., for different personas), you must configure separate `AgentProfile` TOML files with isolated storage paths to prevent database lock contention.