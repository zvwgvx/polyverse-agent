---
title: Worker Contract
summary: The interface that defines how pieces of the system are started, stopped, and connected.
order: 22
---

# Worker Contract

In Polyverse, nearly all functionality is packaged into a `Worker`. A worker is a stateful, asynchronous struct that can be managed by the `Supervisor`. 

The `libs/kernel/src/worker.rs` file defines the trait that unifies the runtime.

## The `Worker` trait

Any piece of the system that needs to run in the background implements this trait:

```rust
#[async_trait]
pub trait Worker: Send + Sync + 'static {
    fn name(&self) -> &str;
    async fn start(&mut self, ctx: WorkerContext) -> Result<()>;
    async fn stop(&mut self) -> Result<()>;
    fn health_check(&self) -> WorkerStatus;
}
```

### 1. `name(&self)`
Returns a static string used for logging and runtime introspection.

### 2. `start(&mut self, ctx: WorkerContext)`
The entry point. This method is called exactly once by the `Supervisor`. It is expected to spawn one or more Tokio tasks that run indefinitely. The worker must return `Ok(())` quickly to signal successful startup; it should *not* block the `start` call.

### 3. `stop(&mut self)`
Called during graceful shutdown. Workers must drop connections, flush state, and terminate their spawned tasks.

### 4. `health_check(&self)`
Called periodically. It returns `WorkerStatus::Healthy`, `Degraded`, `Stopped`, or `NotStarted`.

## The `WorkerContext`

When `start` is called, the worker receives a `WorkerContext`. This is the worker's umbilical cord to the rest of the system:

```rust
pub struct WorkerContext {
    pub event_tx: mpsc::Sender<Event>,
    pub broadcast_rx: broadcast::Sender<Event>,
    pub shutdown: broadcast::Sender<()>,
}
```

- `event_tx`: The outbound queue. Workers use `ctx.emit(Event)` to send an event to the `Coordinator`.
- `broadcast_rx`: The inbound fan-out channel. Workers use `ctx.subscribe_events()` to get a `broadcast::Receiver` that yields all events authorized and rebroadcast by the coordinator.
- `shutdown`: The kill signal. Workers use `ctx.subscribe_shutdown()` to listen for the application-wide exit signal so their background tasks can terminate cleanly.

## Why this contract?

1. **Isolation**: A crashed worker does not directly panic the `Coordinator` or other workers.
2. **Pluggability**: The `apps/agent/src/main.rs` composition root decides which workers to register. To disable Discord, simply skip registering the `DiscordWorker`. To add a new SLM router, write a new worker that listens for `RawEvent`.
3. **Observability**: Because all workers implement `health_check()`, runtime health can be checked in a unified way.