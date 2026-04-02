---
title: Test Harness
summary: How integration tests verify the runtime without network calls.
order: 45
---

# Test Harness

Polyverse enforces a strict testing strategy (`testing/integration-tests`) to verify that the distributed event system works as expected. Because the runtime is event-driven, we can test complex interactions without needing live API keys or active database connections.

## The `MockPlatform`

The `testing/test-support` library provides the crucial `MockPlatform` adapter.

This adapter acts exactly like the `DiscordWorker` or `TelegramWorker`, but instead of connecting to a remote API, it exposes a programmatic channel that test suites can use to push raw text events directly onto the `EventBus` and listen for `ResponseEvent`s coming back out.

## Wiring Verification

The `testing/integration-tests/tests/runtime_wiring.rs` file ensures that the `Supervisor`, `Coordinator`, and core event loop are healthy.

A typical test flow:
1. Spawns the `Supervisor` in-memory.
2. Registers a `MockPlatform` worker.
3. Calls `start_all()`.
4. Pushes a synthetic `RawEvent` ("Hello!") into the `MockPlatform` channel.
5. Asserts that the `Coordinator` logs and rebroadcasts the event correctly across the `broadcast_tx`.
6. Triggers a clean shutdown via the `EventBus` kill signal, asserting that all worker handles terminate properly.

## Subsystem Smoke Tests

Because cognitive workers (like `DialogueEngineWorker`) make actual HTTP calls to LLM APIs, the test harness isolates them via smoke tests (e.g., `dialogue_worker_smoke.rs`).

These tests do not verify *what* the LLM said (which is non-deterministic); instead, they verify that the worker:
- Respects the `is_mention` flag.
- Correctly parses the prompt registry.
- Emits a `ResponseEvent` (or handles API errors gracefully).

## MCP Tests

The `services/mcp` HTTP endpoint is tested end-to-end in `services/mcp/tests/http_contract.rs`.

These tests use Axum's `Router` testing utilities to send `GET` and `POST` requests directly to the in-memory endpoint without opening a real TCP port, validating the JSON schema serialization, HTTP status codes, and `MCP_REQUEST_TIMEOUT_MS` timeout handling.