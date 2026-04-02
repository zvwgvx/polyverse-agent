---
title: Event Model
summary: The shared types and flow that connect sensory input to worker actions.
order: 21
---

# Event Model

Polyverse is a reactive, event-driven runtime. Instead of calling each other directly, workers communicate by broadcasting events onto a shared `EventBus`.

This design decouples sensory input (like Discord messages) from cognitive processing (like LLM dialogue) and state derivation (like tracking user intent).

## Core event flow

1. **Ingestion**: A sensory adapter (e.g., `DiscordWorker` or `TelegramWorker`) receives a message from a platform and broadcasts a `Event::Raw` over the bus.
2. **Rebroadcasting**: The `Coordinator` receives the raw event, logs it, updates its internal state machine, and rebroadcasts it out to all workers.
3. **Processing**: Cognitive workers (like `DialogueEngineWorker` and `AffectEvaluatorWorker`) and state workers independently hear the `Event::Raw` and perform their tasks.
4. **Response**: When a cognitive worker generates a reply, it broadcasts a `Event::Response`.
5. **Egress**: The original sensory adapter hears the `Event::Response`, matches it to the platform/channel, and sends the message back out to the external service.

## Important event types

All events are wrapped in the `libs/kernel/src/event.rs` `Event` enum.

### `Event::Raw(RawEvent)`

The entrypoint for all external input. It normalizes platform differences so that cognitive workers don't need to know if a message came from Discord, Telegram, or the CLI.

Key fields:
- `platform`: Which adapter ingested the event.
- `channel_id` / `message_id`: Routing keys for responses.
- `content`: The normalized text content.
- `is_mention` / `is_dm`: Routing flags for dialogue engine engagement.

### `Event::Response(ResponseEvent)`

The exit path for generated text. Emitted by the `DialogueEngineWorker` (or other potential responders) when it wants to speak. Sensory workers listen for these and route them to their external APIs.

Key fields:
- `platform` / `channel_id` / `reply_to_message_id`: Routing metadata.
- `content`: The text to send.
- `source`: Diagnostic info (`LocalSLM`, `CloudLLM`, or `Template`).

### `Event::BotTurnCompletion(BotTurnCompletion)`

Emitted after a response has been successfully dispatched or a turn has concluded. It is used to unblock the coordinator or trigger follow-up state changes.

### `Event::Biology(BiologyEvent)`

Emitted when the agent's internal biology state (energy, mood, sleep cycle) changes. This is handled by the `Coordinator` to update the agent's core state and can trigger state workers to log the change.

### `Event::Intent(IntentEvent)`

Represents an intermediate classification of a raw event. Emitted by intent classifiers (currently a planned/stubbed pattern) to attach structured intent and sentiment metadata (`Command`, `Question`, `ChitChat`, etc.) to a raw event before full LLM processing.

### `Event::System(SystemEvent)`

Used for runtime lifecycle management. Examples include `WorkerStarted`, `WorkerStopped`, `WorkerError`, and `ShutdownRequested`. The `Supervisor` and `Coordinator` use these to manage the health and shutdown sequences of the agent.

## Why this model?

By strictly adhering to the event bus, it is trivial to add new platforms (just write a new sensory worker that emits `RawEvent`) or new cognitive capabilities (just write a new worker that listens for `RawEvent` and updates memory or state) without rewriting the core `apps/agent` binary logic.
