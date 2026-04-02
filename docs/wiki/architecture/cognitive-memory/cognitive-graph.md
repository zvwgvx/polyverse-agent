---
title: Cognitive Graph
summary: The deeply nested SurrealDB relationship store.
order: 33
---

# Cognitive Graph

The `CognitiveGraph` (`libs/memory/src/graph.rs`) is the definitive, durable source of truth for the agent's relationships and concepts. It is backed by a local instance of SurrealDB (using a RocksDB engine) in `data/polyverse-agent/graph/`.

## Graph Schema

The graph is composed of Nodes and Edges.

### Nodes
- **`person`**: Represents an individual entity the agent knows. The agent itself has a self-identity node defined by its `AgentProfile.graph_self_id` (e.g., `person:agent`). Other users are dynamically assigned IDs like `person:discord_1234567890`.
- **`concept`**: Abstract ideas or topics the agent has learned about or formed opinions on.

### Edges

Edges are directional links connecting Nodes. They hold floating-point values representing the strength of a relationship.

- **`AttitudesTowards`**: Directed from one `person` to another `person` (or `concept`). Contains the core relational metrics:
  - `affinity` (-1.0 to 1.0)
  - `trust` (0.0 to 1.0)
  - `safety` (0.0 to 1.0)
  - `tension` (0.0 to 1.0)

- **`IllusionOf`**: Represents what one person *believes* another person thinks of them. For instance, the agent might have high `affinity` for a user (`AttitudesTowards`), but mistakenly believe the user hates them (`IllusionOf` with negative `affinity`). This creates complex, asymmetrical social dynamics.

## The Write Path

The `AffectEvaluatorWorker` is the primary writer to the `CognitiveGraph`. After analyzing a conversation snippet, it generates a JSON delta representing how the interaction changed the relationship.

The Graph receives these deltas and updates the corresponding edge values. If a node or edge doesn't exist, it creates it.

## Graph vs Tree Projection

Because traversing nested graph edges (e.g., fetching `person:agent`'s `AttitudesTowards` a user, then joining the user's `AttitudesTowards` the agent, then joining the `IllusionOf` edge) is computationally expensive on every chat message, the system uses a **Social Tree Snapshot**.

The tree is a flattened, read-only cache derived from the Graph's complex edges. See [Social Query Model](./social-query-model.md) for how this read path operates in practice.