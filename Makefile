SHELL := /bin/bash

CARGO ?= cargo
TRUNK ?= trunk
TARGET ?= wasm32-unknown-unknown

.PHONY: help serve serve-ui run build check test fmt clippy clean

help:
	@echo "Usage: make <target>"
	@echo
	@echo "Targets:"
	@echo "  serve    Build the frontend and run the single server binary"
	@echo "  run      Alias for serve"
	@echo "  serve-ui Start only the Trunk dev server"
	@echo "  build    Build the web app into dist/"
	@echo "  check   Type-check the Rust code for $(TARGET)"
	@echo "  test    Run cargo tests"
	@echo "  fmt     Format Rust code"
	@echo "  clippy  Run clippy for $(TARGET)"
	@echo "  clean   Remove cargo and trunk build output"

serve:
	@$(TRUNK) build
	@$(CARGO) run --bin robodeck

serve-ui:
	@$(TRUNK) serve

run: serve

build:
	@$(TRUNK) build

check:
	@$(CARGO) check --target $(TARGET)

test:
	@$(CARGO) test

fmt:
	@$(CARGO) fmt

clippy:
	@$(CARGO) clippy --target $(TARGET) -- -D warnings

clean:
	@rm -rf dist target
