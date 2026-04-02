---
title: Core Runtime
summary: The pure infrastructure engine of the system.
order: 10
---

# Core Runtime

This layer is pure computer science infrastructure. It knows nothing about AI or biology. It only handles concurrent execution, message passing, and worker lifecycle.

- [Runtime Primitives](./runtime-primitives.md) — EventBus, Supervisor, and Coordinator.
- [Worker Contract](./worker-contract.md) — The `Trait` interface for modules.
- [Event Model](./event-model.md) — The standard message passing structure.
- [Runtime Flow](./runtime-flow.md) — Bootstrapping and execution lifecycle.
