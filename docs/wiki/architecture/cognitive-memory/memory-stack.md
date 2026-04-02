---
title: Memory Stack
summary: Current memory layers, their responsibilities, and how social query data fits into the runtime.
order: 21
---

# Memory Stack

The repository does not use a single memory system. Instead, the runtime uses multiple layers, each with a different responsibility.

## Current layers

### 1. SQLite message persistence

The persistent chat log is stored through the memory worker and backed by SQLite.

Use this layer for:

- durable message history
- raw mention and reply logs

### 2. Short-term memory

Short-term memory keeps active conversational context in process.

Use this layer for:

- recent interaction context needed during the current runtime session
- fast access for model-facing workers

### 3. Episodic memory

Episodic memory stores compressed semantic recollections in LanceDB.

Supporting pieces are initialized in the composition root:

- `EpisodicStore`
- `MemoryEmbedder`
- `SemanticCompressor`

This layer is intentionally separate from the raw message log.

### 4. Cognitive graph

The cognitive graph stores relationship and affect dynamics in SurrealDB.

This is the current write backbone for social information.

### 5. Social tree snapshot / query model

The runtime also uses a social tree projection as a read/query model.

Current intent:

- graph remains the write/source-of-truth layer
- social tree acts as a query-friendly read model

This supports the current split where:

- affect retrieval is always context-aware
- dialogue retrieval is more selective and gate-driven

## How the layers fit together

A useful way to think about the stack is:

- message persistence stores what was said
- short-term memory keeps recent context in working memory
- episodic memory stores compressed recollections
- cognitive graph stores relationship dynamics
- social tree snapshot supports query-oriented social retrieval

## `MemoryWorker` role

`MemoryWorker` is the bridge across several of these layers.

It listens to broadcast events and keeps the memory systems in sync. Persistent writes and episodic ingestion are intentionally decoupled rather than fused into one path.

## Social query model and MCP

The current social retrieval path is intentionally split into read and write concerns:

- graph = source of truth for writes
- social tree snapshot = query/read model
- `libs/cognitive/src/social_context.rs` = tree-first query facade with fallback metadata

The local MCP service builds on that query layer and currently exposes two read-only tools:

- `social.get_affect_context`
- `social.get_dialogue_summary`

Tests in `services/mcp/tests/` and `testing/integration-tests/tests/social_mcp_roundtrip.rs` provide concrete evidence for this current behavior.

## What not to overclaim

The repository also contains design documents for broader future memory systems, including truth-oriented graph ideas and larger state-space designs. Those are useful background, but they should not be treated as the current implemented stack.

In particular:

- `docs/truth-kgraph-v1.md` is design direction, not current runtime behavior
- `docs/agent-state-space-dimensions.md` is a design document, not the public source of truth for the current implementation

## Where to read next

For current implementation details, prefer:

- `CLAUDE.md`
- `apps/agent/src/main.rs`
- `libs/memory/`
- `libs/cognitive/src/social_context.rs`
- `services/mcp/tests/`
