//! Implementation of the `zad commands [name] [--examples]` subcommand
//! mandated by `OSS_SPEC.md` §12.4.
//!
//! The command enumerates every CLI surface by walking the clap
//! `Command` tree — the same source of truth that `--help`, `--help-agent`,
//! and the manpage-parity test consume — so this output cannot drift
//! from the parser.

use std::fmt::Write as _;

use clap::{Arg, Args, Command as ClapCommand, CommandFactory};

use super::Cli;

#[derive(Debug, Args)]
pub struct CommandsArgs {
    /// Narrow the listing to a single command path, e.g. `discord send` or
    /// `service list`.
    #[arg(num_args = 0.., value_name = "NAME")]
    pub name: Vec<String>,

    /// Print a realistic example invocation for each matching command.
    #[arg(long)]
    pub examples: bool,

    /// Emit a machine-readable JSON dump of every command (path,
    /// description, flags, positionals, example). Consumed by the
    /// website extractor.
    #[arg(long)]
    pub json: bool,
}

pub fn run(args: CommandsArgs) -> crate::error::Result<()> {
    let root = Cli::command();

    if args.json {
        let dump = json_dump(&root);
        println!("{dump}");
        return Ok(());
    }

    let mut out = String::new();
    if args.name.is_empty() {
        if args.examples {
            render_all_examples(&root, &mut out);
        } else {
            render_index(&root, &mut out);
        }
    } else {
        let path: Vec<&str> = args.name.iter().map(String::as_str).collect();
        match find(&root, &path) {
            Some(cmd) => {
                if args.examples {
                    render_command_examples(cmd, &path, &mut out);
                } else {
                    render_command_detail(cmd, &path, &mut out);
                }
            }
            None => {
                return Err(crate::error::ZadError::Invalid(format!(
                    "no such command: `{}`. Run `zad commands` to list available commands.",
                    args.name.join(" ")
                )));
            }
        }
    }

    print!("{out}");
    Ok(())
}

fn render_index(root: &ClapCommand, out: &mut String) {
    let binary = root.get_name().to_string();
    let _ = writeln!(out, "{binary} — available commands");
    out.push('\n');
    walk(root, &mut Vec::new(), &mut |path, cmd| {
        if path.is_empty() {
            return;
        }
        let desc = cmd.get_about().map(|s| s.to_string()).unwrap_or_default();
        let joined = path.join(" ");
        let _ = writeln!(out, "  {binary} {joined:<28} {desc}");
    });
}

fn render_command_detail(cmd: &ClapCommand, path: &[&str], out: &mut String) {
    let _ = writeln!(out, "zad {}", path.join(" "));
    if let Some(about) = cmd.get_about() {
        let _ = writeln!(out, "  {about}");
    }
    out.push('\n');

    let flags: Vec<&Arg> = cmd
        .get_arguments()
        .filter(|a| !a.is_positional() && !a.is_hide_set())
        .collect();
    if !flags.is_empty() {
        out.push_str("Flags:\n");
        for arg in flags {
            let render = format_flag(arg);
            let help = arg.get_help().map(|s| s.to_string()).unwrap_or_default();
            let _ = writeln!(out, "  {render:<32} {help}");
        }
        out.push('\n');
    }

    let positionals: Vec<&Arg> = cmd.get_arguments().filter(|a| a.is_positional()).collect();
    if !positionals.is_empty() {
        out.push_str("Arguments:\n");
        for arg in positionals {
            let name = arg.get_id().as_str();
            let help = arg.get_help().map(|s| s.to_string()).unwrap_or_default();
            let _ = writeln!(out, "  {name:<32} {help}");
        }
        out.push('\n');
    }

    let subs: Vec<&ClapCommand> = cmd.get_subcommands().filter(|s| !s.is_hide_set()).collect();
    if !subs.is_empty() {
        out.push_str("Subcommands:\n");
        for sub in subs {
            let name = sub.get_name();
            let desc = sub.get_about().map(|s| s.to_string()).unwrap_or_default();
            let _ = writeln!(out, "  {name:<20} {desc}");
        }
        out.push('\n');
    }

    out.push_str("Exit codes: 0 success; 1 on any error.\n");
    out.push('\n');
    out.push_str("See `zad man ");
    out.push_str(path.first().copied().unwrap_or("main"));
    out.push_str("` for the full reference.\n");
}

fn render_all_examples(root: &ClapCommand, out: &mut String) {
    let binary = root.get_name();
    let _ = writeln!(out, "{binary} — realistic example invocations");
    out.push('\n');
    walk(root, &mut Vec::new(), &mut |path, _cmd| {
        if path.is_empty() {
            return;
        }
        if let Some(ex) = example_for(path) {
            let _ = writeln!(out, "# {}", path.join(" "));
            let _ = writeln!(out, "{ex}");
            out.push('\n');
        }
    });
}

fn render_command_examples(_cmd: &ClapCommand, path: &[&str], out: &mut String) {
    match example_for(path) {
        Some(ex) => {
            let _ = writeln!(out, "# {}", path.join(" "));
            let _ = writeln!(out, "{ex}");
        }
        None => {
            let _ = writeln!(
                out,
                "no example registered for `{}` — see `zad man {}`.",
                path.join(" "),
                path.first().copied().unwrap_or("main")
            );
        }
    }
}

fn format_flag(arg: &Arg) -> String {
    let mut s = String::new();
    if let Some(short) = arg.get_short() {
        let _ = write!(s, "-{short}");
    }
    if let Some(long) = arg.get_long() {
        if !s.is_empty() {
            s.push_str(", ");
        }
        let _ = write!(s, "--{long}");
    }
    if arg.get_action().takes_values() {
        let _ = write!(s, " <{}>", arg.get_id().as_str().to_uppercase());
    }
    s
}

fn walk<'a>(
    cmd: &'a ClapCommand,
    path: &mut Vec<&'a str>,
    f: &mut dyn FnMut(&[&str], &ClapCommand),
) {
    f(path, cmd);
    for sub in cmd.get_subcommands() {
        if sub.is_hide_set() {
            continue;
        }
        path.push(sub.get_name());
        walk(sub, path, f);
        path.pop();
    }
}

fn find<'a>(root: &'a ClapCommand, path: &[&str]) -> Option<&'a ClapCommand> {
    let mut cur = root;
    for seg in path {
        cur = cur.get_subcommands().find(|s| s.get_name() == *seg)?;
    }
    Some(cur)
}

fn json_dump(root: &ClapCommand) -> String {
    let binary = root.get_name().to_string();
    let version = crate::version();
    let mut commands = Vec::new();
    walk(root, &mut Vec::new(), &mut |path, cmd| {
        if path.is_empty() {
            return;
        }
        let flags: Vec<serde_json::Value> = cmd
            .get_arguments()
            .filter(|a| !a.is_positional() && !a.is_hide_set())
            .map(|a| {
                serde_json::json!({
                    "name": a.get_id().as_str(),
                    "long": a.get_long(),
                    "short": a.get_short().map(|c| c.to_string()),
                    "help": a.get_help().map(|s| s.to_string()),
                    "takes_value": a.get_action().takes_values(),
                })
            })
            .collect();
        let positionals: Vec<serde_json::Value> = cmd
            .get_arguments()
            .filter(|a| a.is_positional())
            .map(|a| {
                serde_json::json!({
                    "name": a.get_id().as_str(),
                    "help": a.get_help().map(|s| s.to_string()),
                })
            })
            .collect();
        commands.push(serde_json::json!({
            "path": path,
            "description": cmd.get_about().map(|s| s.to_string()),
            "flags": flags,
            "positionals": positionals,
            "example": example_for(path),
        }));
    });
    let dump = serde_json::json!({
        "binary": binary,
        "version": version,
        "commands": commands,
    });
    serde_json::to_string_pretty(&dump).unwrap()
}

/// Hand-curated example table keyed by full command path. Add entries
/// next to the clap definitions when new commands land.
fn example_for(path: &[&str]) -> Option<&'static str> {
    match path {
        ["service"] => Some("zad service list"),
        ["service", "list"] => Some("zad service list"),
        ["service", "discord"] => Some("zad service discord add"),
        ["discord"] => Some("zad discord channels"),
        ["discord", "send"] => Some("zad discord send --channel general --body 'deploy complete'"),
        ["discord", "read"] => Some("zad discord read --channel general --limit 20"),
        ["discord", "channels"] => Some("zad discord channels"),
        ["discord", "join"] => Some("zad discord join --channel release-notes"),
        ["discord", "leave"] => Some("zad discord leave --channel release-notes"),
        ["discord", "permissions"] => Some("zad discord permissions show"),
        ["telegram"] => Some("zad telegram chats"),
        ["telegram", "send"] => Some("zad telegram send --chat team-room 'deploy complete'"),
        ["telegram", "read"] => Some("zad telegram read --chat team-room --limit 20"),
        ["telegram", "chats"] => Some("zad telegram chats"),
        ["telegram", "discover"] => Some("zad telegram discover"),
        ["telegram", "directory"] => Some("zad telegram directory"),
        ["telegram", "permissions"] => Some("zad telegram permissions show"),
        ["commands"] => Some("zad commands discord"),
        ["docs"] => Some("zad docs architecture"),
        ["man"] => Some("zad man discord"),
        _ => None,
    }
}
