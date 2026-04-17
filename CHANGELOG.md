# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

This file is **auto-generated from conventional commits at release time** —
do not edit manually.

## [Unreleased]

## [v0.1.0] – 2026-04-17

### Added

- `zad service create <service>` — register global or project-local service
  credentials (stored in the OS keychain, never in TOML).
- `zad service add <service>` — enable a registered service for the current
  project directory.
- Discord service (`--application-id`, `--bot-token-env`, `--scopes`) as the
  first bundled integration.
- Global (`~/.zad/`) and project-local (`~/.zad/projects/<slug>/`) config
  directories with TOML schema validation.
- `ZAD_HOME_OVERRIDE` environment variable for hermetic testing.
- `--help-agent` flag that emits machine-readable service documentation for
  LLM consumption.

[v0.1.0]: https://github.com/niclaslindstedt/zad/releases/tag/v0.1.0
