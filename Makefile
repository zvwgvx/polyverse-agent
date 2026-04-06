SHELL := /bin/bash
.SHELLFLAGS := -eu -o pipefail -c

CARGO ?= cargo
NPM ?= npm

AGENT_PACKAGE := agent
AGENT_BIN := polyverse-agent
WIKI_DIR := apps/wiki

.PHONY: help agent discord discord-selfbot telegram wiki wiki-install test typecheck

help:
	@echo "Targets:"
	@echo "  make agent              Run the Rust agent"
	@echo "  make discord            Run the Discord bot service"
	@echo "  make discord-selfbot    Run the Discord selfbot relay service"
	@echo "  make telegram           Run the Telegram bot service"
	@echo "  make wiki               Run the local wiki on 0.0.0.0"
	@echo "  make wiki-install       Install wiki dependencies"
	@echo "  make test               Run Rust tests"
	@echo "  make typecheck          Typecheck wiki"

agent:
	$(CARGO) run -p $(AGENT_PACKAGE) --bin $(AGENT_BIN)

discord:
	$(CARGO) run -p discord --bin discord-service

discord-selfbot:
	$(CARGO) run -p discord-selfbot --bin discord-selfbot-service

telegram:
	$(CARGO) run -p telegram --bin telegram-service

wiki: wiki-install
	cd $(WIKI_DIR) && $(NPM) run dev

wiki-install:
	@if [ ! -d "$(WIKI_DIR)/node_modules" ]; then \
		cd $(WIKI_DIR) && if [ -f package-lock.json ]; then $(NPM) ci; else $(NPM) install; fi; \
	fi

test:
	$(CARGO) test -q

typecheck: wiki-install
	cd $(WIKI_DIR) && $(NPM) run typecheck
