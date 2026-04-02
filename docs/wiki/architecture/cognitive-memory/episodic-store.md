---
title: Episodic Store
summary: How LanceDB handles high-dimensional semantic search and memory summarization.
order: 32
---

# Episodic Store

The `EpisodicStore` (`libs/memory/src/episodic.rs`) is the agent's long-term semantic memory.

When a conversation session times out in the `ShortTermMemory`, the raw messages are summarized, and the summary is embedded into a high-dimensional vector. These vectors allow the agent to recall past facts, themes, and interactions when prompted with conceptually similar messages in the future.

## Storage Backend

Polyverse currently uses `LanceDB` as its embedded vector database, which is initialized locally at `data/polyverse-agent/lancedb/`.

## The `MemoryEmbedder`

Before a summary can be stored, it must be embedded. The `MemoryEmbedder` uses a pre-configured language model (usually identical to the reasoning model in the cognitive layer) to transform the textual summary of a chat session into an array of floats (e.g., `Vec<f32>`).

This embedding captures the semantic meaning of the conversation, allowing the `EpisodicStore` to find "conceptually similar" memories instead of relying on exact keyword matches.

## Searching the Store

When the `DialogueEngineWorker` receives a new message, it performs a similarity search against the `EpisodicStore`.

1. It embeds the current incoming `RawEvent` (or the active `ShortTermMemory` context).
2. It queries LanceDB for the nearest vectors (e.g., `LIMIT 5`).
3. The store returns the textual summaries associated with those vectors.
4. These historical summaries are appended to the system prompt of the Dialogue Engine.

This allows the agent to reference a conversation from three weeks ago if the current topic semantically matches what was discussed then.

## Why Not SQLite?

While SQLite (used by `MemoryStore`) perfectly logs every exact event verbatim, it cannot perform semantic proximity search. LanceDB ensures that retrieving context for the LLM is extremely fast and conceptually relevant, even across thousands of old chat sessions.