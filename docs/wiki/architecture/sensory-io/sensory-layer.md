---
title: Sensory Layer
summary: How platform adapters ingest messages and bridge external APIs to the event bus.
order: 21
---

# Sensory Layer

The sensory layer (`libs/sensory`) is the outer boundary of the agent. It is entirely responsible for converting platform-specific network protocols (REST, WebSockets) into standard `RawEvent`s and dispatching `ResponseEvent`s back to those platforms.

It contains no cognitive logic. It does not know *what* the agent is thinking; it only knows *how* to receive and send text.

## Platform Adapters

The system currently implements three distinct `Worker` adapters:

### 1. `DiscordWorker`
Uses the `serenity` crate to connect to Discord as an official Bot account. 
- It actively filters out messages from other bots to prevent recursive feedback loops.
- It parses standard `<@userid>` mentions to set the `is_mention` flag.

### 2. `SelfbotWsWorker`
Connecting to Discord as a user account is notoriously difficult in pure Rust due to anti-automation defenses. To solve this, `SelfbotWsWorker` spawns a local Node.js child process (`nodejs-selfbot/index.js`) using `discord.js-selfbot-v13`.
- The Rust worker spins up a local `tokio::net::TcpListener` on a specific port (e.g., 8765).
- The Node.js child process connects to this WebSocket to proxy messages in and out of the Rust core.
- If the Node.js script crashes or fails to spawn, the worker logs a warning but allows the rest of the runtime to continue safely.

### 3. `TelegramWorker`
Uses `teloxide` to connect to the Telegram Bot API. It maps Telegram's chat IDs and handles direct message tagging correctly.

## The `SensoryBuffer`

When a message arrives from *any* adapter, it is passed to a `SensoryBuffer` before hitting the main `EventBus`.

Historically, the buffer managed rate-limiting and typing indicators. In the current implementation, **typing detection is intentionally removed** and the buffer forwards `RawEvent`s immediately. This ensures that the `Coordinator` and `DialogueEngineWorker` receive events with zero artificial latency. 

The `DialogueEngineWorker` itself manages its own context chunking if messages arrive in rapid bursts.

## Egress Routing

All sensory workers subscribe to the global `broadcast_tx`. They listen for `Event::Response`. 

When a `ResponseEvent` arrives, the worker checks the `platform` field:
- If `event.platform == Platform::Discord`, only the `DiscordWorker` processes it. The Telegram worker ignores it.
- The worker looks up the `channel_id` and uses its internal HTTP client (or WebSocket proxy) to dispatch the message back to the external world.