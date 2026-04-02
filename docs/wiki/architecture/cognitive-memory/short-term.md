---
title: Short-Term Memory
summary: How the runtime manages the immediate conversational ring-buffer.
order: 31
---

# Short-Term Memory

In Polyverse, the context of the immediate conversation lives entirely in RAM. This layer is implemented by `ShortTermMemory` (`libs/memory/src/short_term.rs`).

Its job is simple: maintain a rolling window of recent messages for every active conversation, and know when to "forget" them so they can be pushed to long-term storage.

## Session Tracking

The `ShortTermMemory` holds a `HashMap` of `Session`s, keyed by a `ConversationKey`.

The `ConversationKey` combines `(platform, channel_id)` to ensure that Discord DMs, Discord guild channels, and Telegram chats are completely isolated from one another.

Each `Session` tracks:
- `messages`: A `Vec<MemoryMessage>` of the recent raw events.
- `last_active`: The timestamp of the last message.
- `started_at`: The timestamp of the first message in the session.
- `already_ingested`: A flag tracking whether these messages have been semantically compressed.

## Adaptive Expiry & Eviction

Unlike a naive array that just keeps the last 20 messages, `ShortTermMemory` uses **adaptive session timeouts**.

By default, the `ShortTermConfig` has a `base_timeout_secs` of `20 * 60` (20 minutes). However, the session expiry formula adapts based on the length of the conversation:

```rust
let adaptive_secs = base_timeout_secs
    + (self.messages.len() as i64 * 60)
        .min(90 * 60 - base_timeout_secs);
```

For every message in the session, the timeout extends by 60 seconds (up to a hard cap of 90 minutes). This mimics human attention: a quick back-and-forth is forgotten quickly, but a deep, long-running conversation holds the agent's attention much longer before "timing out".

## Flushing to Long-Term

When a new message arrives for a session that has expired (i.e., `now - session.last_active > adaptive_secs`), the `ShortTermMemory` evicts the old session.

1. It returns the `expired_messages` vector back to the `MemoryWorker`.
2. It initializes a fresh, empty session for the new message.
3. The `MemoryWorker` takes those expired messages, feeds them into the `SemanticCompressor`, and writes the resulting summary into the LanceDB `EpisodicStore`.

This guarantees that the agent's immediate prompt context stays lean and focused on the current topic, while the broader historical context is safely archived into vector search.