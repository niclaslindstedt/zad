# zad

A Rust CLI that connects AI agents to external services (Discord, GitHub, Slack, etc.) via scoped adapter configurations instead of MCP servers.

[![CI](https://github.com/niclaslindstedt/zad/actions/workflows/ci.yml/badge.svg)](https://github.com/niclaslindstedt/zad/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

## Why?

- Adapters replace per-agent MCP server setup — one config file wires a service to any agent
- Permission files enforce fine-grained scopes (time windows, content filters) beyond what the upstream API offers
- `--help-agent` flag emits machine-readable docs so an LLM can configure adapters on the user's behalf
- Global (~/.zad/) and project-local configs let teams share defaults while overriding per-repo
- Extending zad with a new provider is a single Rust trait implementation; hooking up services is pure TOML config


## Prerequisites

- _List runtime and dev dependencies with explicit version bounds._

## Install

```sh
# end-to-end install command goes here
```

## Quick start

```sh
# minimal runnable example
```

## Usage

_Reference surface — commands, flags, API entry points._

## Configuration

_Config file paths and key names._

## Examples

See [`examples/`](examples/) for runnable demos.

## Troubleshooting

_Common failure modes and fixes._

## Documentation

- [Getting started](docs/getting-started.md)
- [Configuration](docs/configuration.md)
- [Architecture](docs/architecture.md)
- [Troubleshooting](docs/troubleshooting.md)

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Licensed under [MIT](LICENSE).