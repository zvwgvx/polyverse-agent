---
title: Runtime Primitives
summary: The central hub that manages worker lifecycles and routes events.
order: 23
---

# Runtime Primitives

The core execution model of Polyverse is built on three main primitives living in `libs/runtime/src`: `Supervisor`, `EventBus`, and `Coordinator`.

Together, these control how workers start, how they communicate, and how the system cleanly shuts down.

## `EventBus`

The `EventBus` is the physical transport layer for the application. It holds three channels:

1. **`event_tx` (mpsc)**: A many-to-one queue. Workers use this to send events *inbound* to the `Coordinator`.
2. **`broadcast_tx` (broadcast)**: A one-to-many queue. The `Coordinator` uses this to fan-out events to all workers.
3. **`shutdown_tx` (broadcast)**: A global kill signal that tells all workers to shut down.

When a worker starts, it receives a `WorkerContext` which clones `event_tx`, but forces the worker to `subscribe()` to the broadcast channels, ensuring each worker gets its own cursor for reading broadcast events.

## `Supervisor`

The `Supervisor` owns the `EventBus` and manages worker lifecycles.

In `apps/agent/src/main.rs`, the composition root creates the supervisor and registers workers sequentially:

```rust
let mut supervisor = Supervisor::new();
supervisor.register(DiscordWorker::new(...));
supervisor.register(DialogueEngineWorker::new(...));
// ...
supervisor.start_all().await?;
```

When `start_all()` is called:
1. It sends a `SystemEvent::WorkerStarted` for each worker.
2. It spawns a dedicated Tokio task for each worker by calling its `start()` method with a fresh `WorkerContext`.
3. It tracks the `JoinHandle` of every worker.

During shutdown, the supervisor fires `signal_shutdown()` on the event bus, then waits (with a timeout) for all worker `JoinHandle`s to complete before exiting.

## `Coordinator`

If the `EventBus` is the roads and the `Supervisor` is the city planner, the `Coordinator` is the traffic cop.

It runs in its own background task, exclusively holding the `event_rx` (the receiving end of the mpsc queue). Its job is to consume events sent by workers, inspect them, update global state if necessary, and rebroadcast them.

Key responsibilities:
- **Rebroadcasting**: When a sensory worker emits `Event::Raw`, the coordinator logs it and immediately rebroadcasts it to `broadcast_tx` so all cognitive workers can hear it.
- **State Machine Management**: The coordinator maintains an internal `CoordinatorState` (e.g., `Idle`, `Processing`, `ShuttingDown`).
- **Biology Updates**: When it receives `Event::Biology(BiologyEventKind::EnergyChanged)`, it mutates the agent's core biological state.

The coordinator ensures there is a single, linear history of events that all workers experience simultaneously, rather than a chaotic mesh of workers talking directly to one another.