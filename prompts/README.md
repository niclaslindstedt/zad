# `prompts/`

Versioned LLM prompts. See [`OSS_SPEC.md` §13.5](../OSS_SPEC.md#135-llm-prompts-prompts).

If this project sends prompts to a language model — directly via an SDK
or indirectly via a wrapper — every prompt must live here as a versioned
file rather than as an inline string in source code.

## Layout

```
prompts/
└── <prompt-name>/
    ├── 1_0_0.md
    ├── 1_1_0.md   # new file, non-breaking minor bump
    └── 2_0_0.md   # new file, breaking major bump
```

Versioned files are **immutable** once committed. Every change ships as a
new file at a bumped semver — never edit `1_0_0.md` in place.

## File format

Each `<major>_<minor>_<patch>.md` file is plain Markdown. It must begin
with a YAML front-matter block declaring the prompt's `name`,
`description`, and `version`. The `version` value must match the
filename stem (`1_2_0.md` → `version: 1.2.0`). Loaders strip the front
matter before passing the prompt to the model.

```markdown
---
name: <prompt-name>
description: "<one-sentence description of what this prompt does>"
version: <major>.<minor>.<patch>
---

# <prompt-name>

## System

…system instructions for the model…

## User

…user message body. May contain {{ jinja }} placeholders that the
loader renders with runtime values…
```

The `## System` section is sent verbatim as the system prompt; the
`## User` section is rendered with the project's templating engine and
sent as the user message. The YAML front matter, the `# Title` heading,
and any other prose outside the two required sections are ignored by
the loader and exist purely for humans reading the file.

## Versioning rule

- **Patch bump** (`1_0_0` → `1_0_1`): wording fixes that do not change
  the contract — typos, clarifications.
- **Minor bump** (`1_0_0` → `1_1_0`): non-breaking additions — new
  placeholders, expanded scope, new guidance bullets.
- **Major bump** (`1_x_y` → `2_0_0`): breaking rewrites that callers
  must be updated for — removed placeholders, changed JSON schema,
  fundamentally new task.

Loaders pick the highest version unless explicitly pinned.

If this project performs no LLM calls, leave this directory empty
(this README is enough to satisfy `oss-spec validate`).
