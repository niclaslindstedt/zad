use clap::Args;
use serde::Serialize;

use crate::config;
use crate::error::Result;

const KNOWN_ADAPTERS: &[&str] = &["discord"];

#[derive(Debug, Args)]
pub struct ListArgs {
    /// Emit machine-readable JSON instead of the human-readable table.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Serialize)]
struct ListOutput {
    command: &'static str,
    adapters: Vec<AdapterRow>,
}

#[derive(Debug, Serialize)]
struct AdapterRow {
    name: &'static str,
    global: bool,
    local: bool,
    enabled: bool,
}

pub fn run(args: ListArgs) -> Result<()> {
    let slug = config::path::project_slug()?;
    let project_path = config::path::project_config_path()?;
    let project_cfg = config::load_from(&project_path)?;

    let mut rows = Vec::with_capacity(KNOWN_ADAPTERS.len());
    let mut any_configured = false;
    for name in KNOWN_ADAPTERS {
        let global = config::path::global_adapter_config_path(name)?.exists();
        let local = config::path::project_adapter_config_path_for(&slug, name)?.exists();
        let enabled = project_cfg.has_adapter(name);
        if global || local || enabled {
            any_configured = true;
        }
        rows.push(AdapterRow {
            name,
            global,
            local,
            enabled,
        });
    }

    if args.json {
        let out = ListOutput {
            command: "adapter.list",
            adapters: rows,
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return Ok(());
    }

    let name_w = rows
        .iter()
        .map(|r| r.name.len())
        .chain(std::iter::once("ADAPTER".len()))
        .max()
        .unwrap_or(0);
    let global_w = "GLOBAL".len();
    let local_w = "LOCAL".len();

    println!(
        "{:name_w$}  {:global_w$}  {:local_w$}  PROJECT",
        "ADAPTER", "GLOBAL", "LOCAL"
    );
    for row in &rows {
        println!(
            "{:name_w$}  {:global_w$}  {:local_w$}  {}",
            row.name,
            yes_no(row.global),
            yes_no(row.local),
            if row.enabled { "enabled" } else { "disabled" },
        );
    }

    if !any_configured {
        println!();
        println!("No adapters configured. Run `zad adapter create <adapter>` to start.");
    }

    Ok(())
}

fn yes_no(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}
