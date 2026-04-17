# zad docs

> Print topic documentation embedded into the binary at build time.

## Synopsis

```
zad docs
zad docs <TOPIC>
```

## Description

`zad docs` exposes every file under `docs/*.md` without requiring the
user to know where zad is installed or to browse the source tree. The
Markdown is embedded into the binary at compile time via
`include_str!`, so the exact text a contributor shipped is the exact
text an agent sees at runtime — no filesystem lookup, no version
skew.

| Invocation | Output |
|---|---|
| `zad docs` | Lists every available topic, one per line. |
| `zad docs <TOPIC>` | Prints the full body of `docs/<TOPIC>.md` to stdout. |

Use `zad man` for reference pages about individual commands;
`zad docs` is for conceptual topics (architecture, configuration,
getting started, troubleshooting).

## Options

| Flag | Description |
|---|---|
| `<TOPIC>` | Topic name without the `.md` extension. Omit to list. |

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success. |
| 1 | Unknown topic. |

## See also

`zad man`, `zad commands`, `zad --help-agent`.
