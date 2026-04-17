use crate::config;
use crate::error::Result;

const KNOWN_ADAPTERS: &[&str] = &["discord"];

pub fn run() -> Result<()> {
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
        rows.push((
            *name,
            yes_no(global),
            yes_no(local),
            if enabled { "enabled" } else { "disabled" },
        ));
    }

    let name_w = rows
        .iter()
        .map(|r| r.0.len())
        .chain(std::iter::once("ADAPTER".len()))
        .max()
        .unwrap_or(0);
    let global_w = "GLOBAL".len();
    let local_w = "LOCAL".len();

    println!(
        "{:name_w$}  {:global_w$}  {:local_w$}  PROJECT",
        "ADAPTER", "GLOBAL", "LOCAL"
    );
    for (name, global, local, project) in &rows {
        println!("{name:name_w$}  {global:global_w$}  {local:local_w$}  {project}");
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
