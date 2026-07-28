CARGO ?= cargo

.PHONY: all build release run test test-one snapshot check fmt fmt-check lint clippy ci clean help

all: build

## build: debug build
build:
	$(CARGO) build

## release: optimized build
release:
	$(CARGO) build --release

## run: build and launch ratatop (press q to quit)
run:
	$(CARGO) run

## test: run the whole test suite
test:
	$(CARGO) test

# Cargo filters on substring, so T accepts a bare test name or a module path
# like widgets::braille to run a whole module.
## test-one: run one test or module, e.g. make test-one T=name
test-one:
	@test -n "$(T)" || { echo "usage: make test-one T=<test or module name>"; exit 1; }
	$(CARGO) test $(T) -- --nocapture

## snapshot: print the CPU box as text, no terminal needed
snapshot:
	@$(CARGO) run --quiet --example snapshot

## check: type-check without producing binaries
check:
	$(CARGO) check --all-targets

## fmt: format the source
fmt:
	$(CARGO) fmt

## fmt-check: fail if the source is unformatted
fmt-check:
	$(CARGO) fmt --check

## lint: clippy with warnings as errors
lint:
	$(CARGO) clippy --all-targets -- -D warnings

clippy: lint

## ci: everything the project must pass
ci: fmt-check lint test

## clean: remove build artifacts
clean:
	$(CARGO) clean

## help: list targets
help:
	@grep -hE '^## ' $(MAKEFILE_LIST) | sed 's/## /  /'
