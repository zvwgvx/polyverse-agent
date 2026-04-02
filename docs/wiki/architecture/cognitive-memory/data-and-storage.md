---
title: Data & Storage
summary: Where the runtime stores state, memory, and embeddings locally.
order: 33
---

# Data & Storage

Polyverse is designed to run entirely locally. It does not rely on managed cloud databases (unless configured manually). Instead, it spins up embedded or local databases in its working directory.

## The `data/` Directory

By default, all persistent state is saved under `data/polyverse-agent/` (or whatever path is configured in the loaded `AgentProfile`).

If you need to completely reset the agent's memory, you can simply delete the contents of this folder.

### 1. Persistent Memory (SQLite)
**Default Path:** `data/polyverse-agent/memory.db`

The `MemoryStore` uses an embedded SQLite database to save the raw event stream. This includes every message received and every response generated. It is the permanent log of the agent's life.

### 2. Cognitive Graph (SurrealDB)
**Default Path:** `data/polyverse-agent/graph/`

The `CognitiveGraph` uses an embedded SurrealDB instance (RocksDB backed) to store the deeply nested relationship data. This is where `AffectEvaluatorWorker` writes relationship deltas.

*Note on locks:* SurrealDB holds a strict filesystem lock on this directory while running. If the agent crashes, you may occasionally need to manually remove the `LOCK` file, though graceful shutdown prevents this.

### 3. Episodic Memory (LanceDB)
**Default Path:** `data/polyverse-agent/lancedb/`

The `EpisodicStore` uses LanceDB to store high-dimensional embeddings of semantic memories and older conversation summaries. The agent queries this vector store during dialogue generation to "remember" facts or conversations that have fallen out of the short-term memory buffer.

## Non-Persistent State

Not all data is saved to disk:
- **Short-Term Memory**: The rolling window of the current conversation lives entirely in RAM within `ShortTermMemory`. When the agent restarts, the immediate conversational context is lost (though it can be partially reconstructed from SQLite).
- **Social Tree Projection**: The flat social snapshot used for fast querying is an in-memory cache derived from SurrealDB. It is rebuilt automatically when needed.
- **State Store values**: Currently, the numeric state dimensions (like `emotion.arousal`) only persist their schemas. Depending on the `update_mode`, values may drift or reset entirely on a fresh launch.