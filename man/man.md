# zad man

> Print reference manpages embedded into the binary at build time.

## Synopsis

```
zad man
zad man <COMMAND>
```

## Description

`zad man` exposes every file under `man/*.md` without requiring the
user to know where zad is installed. Unlike `zad docs` (which hosts
conceptual topic guides), `zad man` hosts the command reference —
one page per top-level `zad` command, plus `main` for the overview.

| Invocation | Output |
|---|---|
| `zad man` | Lists every available manpage, one per line. |
| `zad man main` | Overview manpage. |
| `zad man <COMMAND>` | Prints the full body of `man/<COMMAND>.md` to stdout. |

A `tests/manpage_parity_test.rs` integration test asserts that every
top-level clap subcommand has a matching manpage and vice versa, so
this surface cannot silently drift from the parser.

## Options

| Flag | Description |
|---|---|
| `<COMMAND>` | Command name without the `.md` extension. Omit to list. |

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success. |
| 1 | Unknown command. |

## See also

`zad docs`, `zad commands`, `zad --help-agent`.
