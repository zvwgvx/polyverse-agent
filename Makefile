SHELL := /bin/bash
.SHELLFLAGS := -eu -o pipefail -c

CARGO ?= cargo
NPM ?= npm

AGENT_PACKAGE := pa-agent
AGENT_BIN := polyverse-agent
COCKPIT_DIR := apps/cockpit-web

.PHONY: help agent cockpit cockpit-install test typecheck

help:
	@echo "Targets:"
	@echo "  make agent            Run the Rust agent"
	@echo "  make cockpit          Run the local cockpit (Next.js dev)"
	@echo "  make cockpit-install  Install cockpit dependencies"
	@echo "  make test             Run Rust tests"
	@echo "  make typecheck        Typecheck cockpit"

agent:
	$(CARGO) run -p $(AGENT_PACKAGE) --bin $(AGENT_BIN)

cockpit: cockpit-install
	cd $(COCKPIT_DIR) && $(NPM) run dev

cockpit-install:
	@if [ ! -d "$(COCKPIT_DIR)/node_modules" ]; then \
		cd $(COCKPIT_DIR) && if [ -f package-lock.json ]; then $(NPM) ci; else $(NPM) install; fi; \
	fi

test:
	$(CARGO) test -q

typecheck: cockpit-install
	cd $(COCKPIT_DIR) && $(NPM) run typecheck

