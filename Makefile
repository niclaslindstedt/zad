.PHONY: build test lint fmt fmt-check shellcheck actionlint release clean docs website website-dev install bench


build:
	cargo build --workspace

test:
	cargo test --workspace

lint:
	cargo clippy --workspace --all-targets -- -D warnings

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

shellcheck:
	shellcheck scripts/*.sh

# OSS_SPEC §16.1 — lint every workflow YAML with actionlint. Treats
# missing `actionlint` as an installable dependency: developers can
# fetch it from https://github.com/rhysd/actionlint, and CI fetches
# it via the official installer script.
actionlint:
	actionlint -color

release:
	cargo build --workspace --release

clean:
	cargo clean

install:
	cargo install --path crates/zad-cli


docs:
	@echo "see docs/"

website:
	cargo build --bin zad
	cd website && npm install && npm run build

website-dev:
	cargo build --bin zad
	cd website && npm install && npm run dev
