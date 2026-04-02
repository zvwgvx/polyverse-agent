---
title: Social Query Model
summary: How the runtime reads and caches relationship data via the social tree projection.
order: 25
---

# Social Query Model

Understanding the user is critical to the agent's behavior. However, reading and joining complex relationships directly from the graph database for every message is too slow.

To solve this, Polyverse splits social context into a **Write Path** (the Graph) and a **Read Path** (the Social Tree). This architecture is primarily implemented in `libs/cognitive/src/social_context.rs`.

## Graph vs. Tree

1. **The Cognitive Graph (SurrealDB)**
   - The source of truth.
   - Stores raw interactions, individual deltas, and deeply nested relationship edges (e.g., `AttitudesTowards`, `IllusionOf`).
   - Slow to query fully, but highly durable.
   - Written to by the `AffectEvaluatorWorker`.

2. **The Social Tree Snapshot**
   - A materialized projection (or "read model") of the graph.
   - Flattens the complex graph into a single JSON document per user containing core metrics (`affinity`, `trust`, `tension`) and natural language summaries.
   - Stored in memory and the fast local cache.
   - Extremely fast to query.

## Querying the Tree

When a worker (like the Dialogue Engine) needs context, it uses the facade function `query_social_context()`.

It must specify an intent:
- `SocialQueryIntent::DialogueSummary`: Used by the dialogue engine. Asks for a lightweight text summary and coarse trust/tension states.
- `SocialQueryIntent::AffectRich`: Used by the affect evaluator. Asks for exact floating-point metrics (e.g., `affinity: 0.82`) to calculate precise deltas.

## Freshness and Fallbacks

Because the tree is a cache, the query facade enforces strict staleness rules. When a worker requests context, the system returns a `SocialQueryResult` which includes the `SocialQuerySource` metadata:

- `tree_fresh`: The tree snapshot was found and is newer than the `max_staleness_ms` limit.
- `tree_stale`: The snapshot was found but is too old. The system triggers a background re-projection from the graph, but immediately returns the stale data to keep latency low.
- `graph_fallback`: No tree snapshot existed. The system blocks, queries the graph directly, computes the snapshot, and returns it.
- `default_fallback`: The user is completely new or the graph is empty. The system returns baseline zeros.

## MCP Integration

This exact query pipeline is exposed to LLMs via the MCP transport. The `social.get_affect_context` and `social.get_dialogue_summary` tools map directly to the intents above, meaning the LLM gets the exact same view of the user that the native workers do.