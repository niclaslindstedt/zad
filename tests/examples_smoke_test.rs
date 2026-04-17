//! OSS_SPEC §13: every example in `examples/` must parse and load via
//! the same code path the production CLI uses. A loose-TOML example
//! that silently stops parsing is worse than no example — this test
//! makes sure `make test` fails the moment an example rots.

use std::path::PathBuf;

use zad::service::discord::permissions;

fn example_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join(rel)
}

#[test]
fn discord_permissions_example_loads() {
    let path = example_path("discord-permissions/permissions.toml");
    assert!(
        path.exists(),
        "example file missing at {} — did the §13 restructure get reverted?",
        path.display()
    );

    let loaded = permissions::load_file(&path)
        .expect("example permissions file must parse through the production loader");
    assert!(
        loaded.is_some(),
        "load_file returned Ok(None) for an existing example file at {}",
        path.display()
    );
}
