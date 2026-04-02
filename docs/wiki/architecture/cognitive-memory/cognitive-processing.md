---
title: Cognitive Layer
summary: The interface between LLM-powered workers and the rest of the runtime.
order: 24
---

# Cognitive Layer

The `libs/cognitive` package bridges the deterministic event system and non-deterministic LLM operations. Rather than treating "the AI" as a monolith, Polyverse splits cognitive capabilities into distinct workers and models.

## `DialogueEngineWorker`

This worker is responsible for generating the actual text responses that users see.

**Flow:**
1. Listens to `Event::Raw`.
2. Ignores events that don't mention the agent or aren't direct messages (unless configured otherwise).
3. Gathers context: short-term chat history, episodic memory retrieval, graph context (via `SocialQueryIntent::DialogueSummary`), and optional numeric state snapshots.
4. Renders a system prompt using `kernel::prompt_registry::render_prompt_or()`.
5. Calls the LLM (e.g., Claude or OpenAI).
6. Interprets the result (handling tool loops if the model uses tools like `social.get_affect_context`).
7. Broadcasts `Event::Response` when final text is generated.

## `AffectEvaluatorWorker`

This worker is responsible for updating the agent's internal emotional and relationship state *after* every interaction. It is structurally decoupled from the dialogue engine.

**Flow:**
1. Listens to `Event::Raw`.
2. Fetches the high-fidelity relationship context (via `SocialQueryIntent::AffectRich`).
3. Calls a separate, usually smaller/faster LLM to evaluate:
   - What was the sentiment?
   - How did affinity, trust, safety, or tension change?
   - Did the user issue a command or make a request?
4. Emits `Event::Intent` for downstream state workers.
5. Emits `Event::Biology` if mood/energy changed.
6. Writes directly to the `CognitiveGraph` to persist the relationship delta.

## Why the split?

Decoupling dialogue generation from affect evaluation has three massive benefits:
1. **Speed**: The dialogue engine doesn't have to output massive JSON relationship updates before it can say "Hello!".
2. **Quality**: The dialogue engine focuses entirely on persona and conversation, while the affect evaluator focuses entirely on structural analysis.
3. **Safety**: If the affect evaluator fails or hallucinates invalid JSON, the user still gets a response.

## Prompt Registry

Cognitive workers do not hardcode their prompts. They use logical IDs like `system.dialogue` or `context.social.known`.

The `config/prompt_registry.json` maps these IDs to physical markdown files under `prompts/`. This allows prompts to be edited without recompiling the Rust binary.