use zad::config::{self, DiscordServiceCfg, ProjectConfig};

#[test]
fn project_config_roundtrips_an_enabled_discord_service() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("config.toml");

    let mut cfg = ProjectConfig::default();
    cfg.enable_discord();

    config::save_to(&path, &cfg).unwrap();
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(body.contains("[service.discord]"), "body was:\n{body}");
    assert!(body.contains("enabled = true"), "body was:\n{body}");
    assert!(
        !body.contains("application_id"),
        "project config must not hold creds"
    );

    let reloaded = config::load_from(&path).unwrap();
    assert_eq!(reloaded, cfg);
}

#[test]
fn global_discord_config_serializes_flat() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("global.toml");

    let cfg = DiscordServiceCfg {
        application_id: "1234".to_string(),
        scopes: vec!["guilds".into()],
        default_guild: Some("987".into()),
    };

    config::save_flat(&path, &cfg).unwrap();
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(
        !body.contains("[service"),
        "global config should be flat, got:\n{body}"
    );
    assert!(body.contains("application_id = \"1234\""));

    let reloaded: DiscordServiceCfg = config::load_flat(&path).unwrap().unwrap();
    assert_eq!(reloaded, cfg);
}

#[test]
fn missing_project_file_loads_as_default() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("does-not-exist.toml");
    let cfg = config::load_from(&path).unwrap();
    assert!(!cfg.has_service("discord"));
}

#[test]
fn missing_global_file_loads_as_none() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("missing.toml");
    let v: Option<DiscordServiceCfg> = config::load_flat(&path).unwrap();
    assert!(v.is_none());
}

#[test]
fn has_service_returns_false_when_empty() {
    let cfg = ProjectConfig::default();
    assert!(!cfg.has_service("discord"));
    assert!(cfg.discord().is_none());
}
