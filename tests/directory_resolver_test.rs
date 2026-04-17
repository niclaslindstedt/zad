//! Unit tests for [`zad::config::directory::Directory`] resolution rules.
//! These exercise the name-lookup logic without going through a spawned
//! binary — they live in a separate file so they can be run quickly and
//! without the keychain/ZAD_HOME setup the CLI tests need.

use zad::config::directory::Directory;

fn sample() -> Directory {
    let mut d = Directory::default();
    d.guilds.insert("main".to_string(), "100".to_string());
    d.guilds.insert("other".to_string(), "200".to_string());
    d.channels.insert("general".to_string(), "11".to_string());
    d.channels
        .insert("main/general".to_string(), "11".to_string());
    d.channels
        .insert("other/general".to_string(), "22".to_string());
    d.users.insert("alice".to_string(), "1001".to_string());
    d
}

#[test]
fn numeric_input_is_passed_through() {
    let d = sample();
    assert_eq!(d.resolve_channel("123456789", None), Some(123456789));
    assert_eq!(d.resolve_user("987"), Some(987));
    assert_eq!(d.resolve_guild("555"), Some(555));
}

#[test]
fn hash_prefix_is_stripped_for_channels() {
    let d = sample();
    assert_eq!(d.resolve_channel("#general", None), Some(11));
}

#[test]
fn at_prefix_is_stripped_for_users() {
    let d = sample();
    assert_eq!(d.resolve_user("@alice"), Some(1001));
}

#[test]
fn qualified_channel_lookup_wins_over_bare() {
    let d = sample();
    // Bare `general` exists but the qualified form disambiguates.
    assert_eq!(d.resolve_channel("other/general", None), Some(22));
}

#[test]
fn context_guild_disambiguates_bare_channel_name() {
    let d = sample();
    // Bare key `general` maps to 11; with `other` as the context guild,
    // the `other/general` qualified entry wins.
    assert_eq!(d.resolve_channel("general", Some("other")), Some(22));
}

#[test]
fn context_guild_falls_back_to_bare_key_when_not_qualified() {
    let d = sample();
    // No `main/announcements` entry; we only have the bare `general`
    // and its `main/general` mirror. Asking for a name that's only in
    // the bare map should still resolve.
    let mut d = d;
    d.channels.insert("lobby".to_string(), "99".to_string());
    assert_eq!(d.resolve_channel("lobby", Some("main")), Some(99));
}

#[test]
fn unknown_name_returns_none() {
    let d = sample();
    assert!(d.resolve_channel("ghost", None).is_none());
    assert!(d.resolve_user("nobody").is_none());
    assert!(d.resolve_guild("missing").is_none());
}

#[test]
fn guild_name_for_is_a_reverse_lookup() {
    let d = sample();
    assert_eq!(d.guild_name_for(100), Some("main"));
    assert_eq!(d.guild_name_for(200), Some("other"));
    assert_eq!(d.guild_name_for(999), None);
}

#[test]
fn toml_round_trip_preserves_entries() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("directory.toml");
    let written = sample();
    zad::config::directory::save_to(&path, &written).unwrap();
    let read_back = zad::config::directory::load_from(&path).unwrap();
    assert_eq!(written, read_back);
}
