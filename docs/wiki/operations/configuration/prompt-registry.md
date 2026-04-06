---
title: Prompt Registry
summary: How the template system maps logical IDs to text files and supports hot-reloading.
order: 35
---

# Prompt Registry

Cognitive workers do not hardcode the exact text they send to language models. Instead, Polyverse implements a `PromptRegistry` (`libs/kernel/src/prompt_registry.rs`) that maps logical prompt IDs to physical Markdown files on disk.

This decoupling allows prompt engineers to tune the agent's behavior without touching Rust code or restarting the binary.

## How it Works

The central configuration file is `config/prompt_registry.json`.

It defines a JSON object where keys are logical IDs (e.g., `system.dialogue`, `context.social.known`) and values are the paths to the physical `.md` or `.txt` files containing the raw prompt (e.g., `prompts/system_dialogue.md`).

## Templating Engine

When a cognitive worker (like the `DialogueEngineWorker`) needs to build a context window, it calls `render_prompt_or(id, replacements, fallback)`.

The `replacements` are key-value pairs (e.g., `("username", "John")`). 

The registry loads the text file associated with the `id`, replaces all instances of `{{username}}` with `"John"`, and returns the final rendered string. If the file is missing or the ID is unregistered, it gracefully falls back to the hardcoded `fallback` text.

## Prompt Updates

Because prompts are externalized to files and a registry, updates can be made directly by editing the mapped files under `prompts/` and adjusting `config/prompt_registry.json` when needed.

The runtime resolves prompt text on demand, so the next turn will use the updated content from disk.