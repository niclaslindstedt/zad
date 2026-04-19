.PHONY: build test lint fmt fmt-check release clean docs pages pages-dev install bench


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

release:
	cargo build --release

clean:
	cargo clean

install:
	cargo install --path .


docs:
	@echo "see docs/"

pages:
	cd pages && npm install && npm run build

pages-dev:
	cd pages && npm install && npm run dev