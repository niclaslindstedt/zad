//! `--help-agent` renderer for `zad discord`. Emits a single JSON
//! document that teaches an automation everything it needs to drive
//! every Discord verb: preconditions (project enablement, credentials),
//! per-verb flags / positional args / defaults / types, representative
//! examples, JSON-output contracts, and exit-code semantics.
//!
//! Flag shape is introspected from clap's command tree so it cannot
//! drift from the actual CLI. Semantic context (auth model, concepts,
//! examples, output shapes) is hand-authored because clap only knows
//! syntactic metadata.

use clap::{ArgAction, CommandFactory};
use serde::Serialize;

use crate::cli::Cli;
use crate::error::Result;

#[derive(Debug, Serialize)]
struct Document {
    command: &'static str,
    version: &'static str,
    summary: &'static str,
    auth: AuthModel,
    preconditions: Vec<&'static str>,
    concepts: Concepts,
    global_flags: Vec<FlagDoc>,
    verbs: Vec<Verb>,
    output_modes: OutputModes,
    exit_codes: Vec<ExitCode>,
}

#[derive(Debug, Serialize)]
struct AuthModel {
    token_storage: &'static str,
    token_keychain_service: &'static str,
    token_keychain_account_global: &'static str,
    token_keychain_account_local: &'static str,
    scope_resolution: &'static str,
    global_config_path: &'static str,
    local_config_path: &'static str,
    project_config_path: &'static str,
}

#[derive(Debug, Serialize)]
struct Concepts {
    snowflake: &'static str,
    guild: &'static str,
    default_guild_fallback: &'static str,
    body_input: &'static str,
    thread_join_leave: &'static str,
    pagination_cap: &'static str,
}

#[derive(Debug, Serialize)]
struct FlagDoc {
    long: Option<String>,
    short: Option<String>,
    value_name: Option<String>,
    #[serde(rename = "type")]
    kind: String,
    required: bool,
    takes_value: bool,
    default: Option<String>,
    description: String,
}

#[derive(Debug, Serialize)]
struct PositionalDoc {
    name: String,
    required: bool,
    description: String,
}

#[derive(Debug, Serialize)]
struct Verb {
    name: String,
    summary: String,
    usage: &'static str,
    flags: Vec<FlagDoc>,
    positionals: Vec<PositionalDoc>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    notes: Vec<&'static str>,
    examples: Vec<&'static str>,
    json_output: JsonOutput,
}

#[derive(Debug, Serialize)]
struct JsonOutput {
    command_id: &'static str,
    fields: Vec<JsonField>,
}

#[derive(Debug, Serialize)]
struct JsonField {
    name: &'static str,
    #[serde(rename = "type")]
    kind: &'static str,
    description: &'static str,
}

#[derive(Debug, Serialize)]
struct OutputModes {
    human: &'static str,
    json: &'static str,
}

#[derive(Debug, Serialize)]
struct ExitCode {
    code: u8,
    meaning: &'static str,
}

pub fn render() -> Result<String> {
    let root = <Cli as CommandFactory>::command();
    let cmd = root
        .find_subcommand("discord")
        .expect("`discord` subcommand is part of the CLI");

    let global_flags: Vec<FlagDoc> = cmd
        .get_arguments()
        .filter(|a| !a.is_positional() && a.get_id() != "help")
        .map(describe_flag)
        .collect();

    let verbs: Vec<Verb> = cmd
        .get_subcommands()
        .map(|sc| {
            let name = sc.get_name().to_string();
            let summary = sc.get_about().map(|s| s.to_string()).unwrap_or_default();
            let flags: Vec<FlagDoc> = sc
                .get_arguments()
                .filter(|a| !a.is_positional() && a.get_id() != "help")
                .map(describe_flag)
                .collect();
            let positionals: Vec<PositionalDoc> = sc
                .get_arguments()
                .filter(|a| a.is_positional())
                .map(describe_positional)
                .collect();
            augment(name.as_str(), summary, flags, positionals)
        })
        .collect();

    let doc = Document {
        command: "discord",
        version: env!("CARGO_PKG_VERSION"),
        summary: "Runtime operations against a configured Discord bot. \
                  Configuration verbs (create/enable/disable/show/delete) \
                  live under `zad service <action> discord`; this command \
                  group is for day-to-day use by agents and humans.",
        auth: AuthModel {
            token_storage: "OS keychain (macOS Keychain / Linux Secret Service / Windows Credential Manager). Never written to TOML.",
            token_keychain_service: "zad",
            token_keychain_account_global: "discord-bot:global",
            token_keychain_account_local: "discord-bot:<project-slug>",
            scope_resolution: "Local credentials under the current project's slug override global ones. A project must have opted in via `zad service enable discord`.",
            global_config_path: "~/.zad/services/discord/config.toml",
            local_config_path: "~/.zad/projects/<slug>/services/discord/config.toml",
            project_config_path: "~/.zad/projects/<slug>/config.toml",
        },
        preconditions: vec![
            "Credentials registered with `zad service create discord` (global) or `--local` (project).",
            "Current project has opted in via `zad service enable discord` (writes `[service.discord] enabled = true`).",
            "The bot user is already a member of any guild it needs to read, post to, or list channels in — zad does not automate guild joining.",
        ],
        concepts: Concepts {
            snowflake: "A Discord snowflake is a decimal string of digits identifying channels, users, guilds, and messages. Accepted wherever a flag says `<ID>` or `<USER_ID>`.",
            guild: "A Discord server. Channels always belong to a guild.",
            default_guild_fallback: "`zad discord channels` falls back to the `default_guild` key from the effective service config when `--guild` is omitted.",
            body_input: "`zad discord send` takes the message body as a positional argument OR from standard input via `--stdin`. The flags are mutually exclusive.",
            thread_join_leave: "Discord only supports explicit join/leave on *thread* channels (endpoints PUT/DELETE /channels/{id}/thread-members/@me). Regular guild text and voice channels are joined implicitly by virtue of guild membership and channel permissions — the `join`/`leave` verbs error on non-thread IDs.",
            pagination_cap: "`zad discord read --limit` maps to Discord's GET-messages endpoint which caps results at 100 per request. Higher values are silently clamped.",
        },
        global_flags,
        verbs,
        output_modes: OutputModes {
            human: "Default mode prints human-friendly text on stdout. Error messages always go to stderr.",
            json: "Passing `--json` to any verb switches stdout to pretty-printed JSON. Every JSON object has a `command` field identifying the verb (e.g. `discord.send`).",
        },
        exit_codes: vec![
            ExitCode {
                code: 0,
                meaning: "Success.",
            },
            ExitCode {
                code: 1,
                meaning: "Runtime error — Discord API rejected the request, keychain read failed, project not enabled, credentials missing, etc.",
            },
            ExitCode {
                code: 2,
                meaning: "Usage error — clap rejected the invocation (unknown flag, conflicting flags, missing required argument, non-numeric snowflake).",
            },
        ],
    };

    Ok(serde_json::to_string_pretty(&doc).unwrap())
}

fn describe_flag(arg: &clap::Arg) -> FlagDoc {
    let long = arg.get_long().map(|s| format!("--{s}"));
    let short = arg.get_short().map(|c| format!("-{c}"));
    let takes_value = matches!(arg.get_action(), ArgAction::Set | ArgAction::Append);
    let value_name = if takes_value {
        arg.get_value_names()
            .and_then(|v| v.first().map(|s| s.to_string()))
    } else {
        None
    };
    let kind = if !takes_value {
        "bool".to_string()
    } else {
        infer_type(arg.get_id().as_str(), value_name.as_deref())
    };
    let default = arg
        .get_default_values()
        .first()
        .map(|v| v.to_string_lossy().into_owned());
    FlagDoc {
        long,
        short,
        value_name,
        kind,
        required: arg.is_required_set(),
        takes_value,
        default,
        description: arg.get_help().map(|s| s.to_string()).unwrap_or_default(),
    }
}

fn describe_positional(arg: &clap::Arg) -> PositionalDoc {
    PositionalDoc {
        name: arg
            .get_value_names()
            .and_then(|v| v.first().map(|s| s.to_string()))
            .unwrap_or_else(|| arg.get_id().as_str().to_uppercase()),
        required: arg.is_required_set(),
        description: arg.get_help().map(|s| s.to_string()).unwrap_or_default(),
    }
}

/// Infer a semantic type label from the argument id / value name. Clap
/// only tracks Rust types under the hood; this promotes a few known ids
/// ("channel", "dm", "guild", …) to domain-level labels an agent can
/// reason about.
fn infer_type(id: &str, value_name: Option<&str>) -> String {
    match id {
        "channel" | "dm" | "guild" => "snowflake".to_string(),
        "limit" => "integer".to_string(),
        _ => value_name.unwrap_or("string").to_lowercase(),
    }
}

fn augment(
    name: &str,
    summary: String,
    flags: Vec<FlagDoc>,
    positionals: Vec<PositionalDoc>,
) -> Verb {
    match name {
        "send" => Verb {
            name: name.to_string(),
            summary,
            usage: "zad discord send (--channel <ID> | --dm <USER_ID>) [--stdin] [BODY] [--json]",
            flags,
            positionals,
            notes: vec![
                "Exactly one of `--channel` or `--dm` is required.",
                "The body is required unless `--stdin` is passed.",
                "`--stdin` reads everything from standard input; a trailing newline is stripped.",
                "DM posting creates/reopens the DM channel with the target user automatically.",
            ],
            examples: vec![
                "zad discord send --channel 1111111111111111 \"deploy finished\"",
                "tail -n 20 deploy.log | zad discord send --channel 1111111111111111 --stdin",
                "zad discord send --dm 222222222222222 \"standup in 5 minutes\"",
                "zad discord send --channel 1111111111111111 \"hi\" --json",
            ],
            json_output: JsonOutput {
                command_id: "discord.send",
                fields: vec![
                    JsonField {
                        name: "command",
                        kind: "string",
                        description: "Always `discord.send`.",
                    },
                    JsonField {
                        name: "target",
                        kind: "string",
                        description: "Either `channel` or `dm`.",
                    },
                    JsonField {
                        name: "target_id",
                        kind: "snowflake-string",
                        description: "ID the message was sent to.",
                    },
                    JsonField {
                        name: "message_id",
                        kind: "snowflake-string",
                        description: "ID of the newly created message.",
                    },
                ],
            },
        },
        "read" => Verb {
            name: name.to_string(),
            summary,
            usage: "zad discord read --channel <ID> [--limit N] [--json]",
            flags,
            positionals,
            notes: vec![
                "Human mode prints messages oldest-first so a terminal reader sees chronological order; JSON mode preserves Discord's newest-first ordering.",
                "`--limit` is clamped to Discord's 100-per-request cap.",
            ],
            examples: vec![
                "zad discord read --channel 1111111111111111",
                "zad discord read --channel 1111111111111111 --limit 50",
                "zad discord read --channel 1111111111111111 --limit 10 --json | jq '.messages[].body'",
            ],
            json_output: JsonOutput {
                command_id: "discord.read",
                fields: vec![
                    JsonField {
                        name: "command",
                        kind: "string",
                        description: "Always `discord.read`.",
                    },
                    JsonField {
                        name: "channel",
                        kind: "snowflake-string",
                        description: "Channel ID that was read.",
                    },
                    JsonField {
                        name: "count",
                        kind: "integer",
                        description: "Number of messages returned.",
                    },
                    JsonField {
                        name: "messages",
                        kind: "array",
                        description: "Array of `{id, author, body}` objects. `id` and `author` are snowflake-strings; `body` is UTF-8.",
                    },
                ],
            },
        },
        "channels" => Verb {
            name: name.to_string(),
            summary,
            usage: "zad discord channels [--guild <ID>] [--json]",
            flags,
            positionals,
            notes: vec![
                "Omit `--guild` to use the `default_guild` key from the effective Discord config.",
                "Result covers every channel kind: `text`, `voice`, `category`, `news`, `public_thread`, `private_thread`, `news_thread`, `stage`, `forum`, `directory`.",
                "Output is sorted by channel `position` then by name.",
            ],
            examples: vec![
                "zad discord channels",
                "zad discord channels --guild 999999999999999",
                "zad discord channels --json | jq '.channels[] | select(.kind==\"text\")'",
            ],
            json_output: JsonOutput {
                command_id: "discord.channels",
                fields: vec![
                    JsonField {
                        name: "command",
                        kind: "string",
                        description: "Always `discord.channels`.",
                    },
                    JsonField {
                        name: "guild",
                        kind: "snowflake-string",
                        description: "Guild ID the listing belongs to.",
                    },
                    JsonField {
                        name: "count",
                        kind: "integer",
                        description: "Number of channels returned.",
                    },
                    JsonField {
                        name: "channels",
                        kind: "array",
                        description: "Array of `{id, name, kind, parent?, position}` objects.",
                    },
                ],
            },
        },
        "join" => Verb {
            name: name.to_string(),
            summary,
            usage: "zad discord join --channel <ID> [--json]",
            flags,
            positionals,
            notes: vec![
                "Only thread channels can be joined explicitly; non-thread IDs return a Discord API error.",
            ],
            examples: vec![
                "zad discord join --channel 3333333333333333",
                "zad discord join --channel 3333333333333333 --json",
            ],
            json_output: JsonOutput {
                command_id: "discord.join",
                fields: vec![
                    JsonField {
                        name: "command",
                        kind: "string",
                        description: "Always `discord.join`.",
                    },
                    JsonField {
                        name: "channel",
                        kind: "snowflake-string",
                        description: "Thread channel the bot joined.",
                    },
                ],
            },
        },
        "leave" => Verb {
            name: name.to_string(),
            summary,
            usage: "zad discord leave --channel <ID> [--json]",
            flags,
            positionals,
            notes: vec![
                "Only thread channels can be left explicitly; non-thread IDs return a Discord API error.",
            ],
            examples: vec![
                "zad discord leave --channel 3333333333333333",
                "zad discord leave --channel 3333333333333333 --json",
            ],
            json_output: JsonOutput {
                command_id: "discord.leave",
                fields: vec![
                    JsonField {
                        name: "command",
                        kind: "string",
                        description: "Always `discord.leave`.",
                    },
                    JsonField {
                        name: "channel",
                        kind: "snowflake-string",
                        description: "Thread channel the bot left.",
                    },
                ],
            },
        },
        _ => Verb {
            name: name.to_string(),
            summary,
            usage: "",
            flags,
            positionals,
            notes: vec![],
            examples: vec![],
            json_output: JsonOutput {
                command_id: "discord.unknown",
                fields: vec![],
            },
        },
    }
}
