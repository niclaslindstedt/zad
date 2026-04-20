.PHONY: build test lint fmt fmt-check shellcheck release clean docs website website-dev install bench


build:
	cargo build

test:
	cargo test

lint:
	cargo clippy --all-targets -- -D warnings

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

shellcheck:
	shellcheck scripts/*.sh

release:
	cargo build --release

clean:
	cargo clean

install:
	cargo install --path .


docs:
	@echo "see docs/"

website:
	cargo build --bin zad
	cd website && npm install && npm run build

website-dev:
	cargo build --bin zad
	cd website && npm install && npm run dev