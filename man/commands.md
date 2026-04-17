# zad commands

> Enumerate the CLI surface — list every command, inspect a single
> command's flags and exit codes, or emit realistic example
> invocations. Driven by the same clap tree that powers `--help` and
> `--help-agent`, so the output cannot drift from the parser.

## Synopsis

```
zad commands
zad commands <NAME>...
zad commands --examples
zad commands <NAME>... --examples
zad commands --json
```

## Description

`zad commands` is the primary machine-friendly discovery surface
mandated by OSS_SPEC.md §12.4. It has four modes:

| Invocation | Output |
|---|---|
| `zad commands` | Tree of every command with a one-line description. |
| `zad commands <NAME>...` | Full reference for one command: flags, positionals, subcommands, exit codes, pointer to the matching manpage. |
| `zad commands --examples` | Realistic example invocation for every command that has one. |
| `zad commands <NAME>... --examples` | Example for a single command. |
| `zad commands --json` | Machine-readable dump consumed by the website extractor and by external tooling. |

## Options

| Flag | Description |
|---|---|
| `<NAME>...` | One or more space-separated name segments identifying a command path (e.g. `discord send`). When omitted, all commands are listed. |
| `--examples` | Print hand-curated example invocations instead of the flag reference. |
| `--json` | Emit a JSON document with every command's path, description, flags, positionals, and example. |

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success. |
| 1 | Unknown command path, or any other error. |

## See also

`zad --help-agent`, `zad man`, `zad docs`.
