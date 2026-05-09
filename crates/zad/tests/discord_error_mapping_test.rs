//! Unit tests for the HTTP status/code → typed `ZadError` decision table
//! exposed by `DiscordHttp`. We can't portably construct a
//! `serenity::Error` from outside the crate (most variants are
//! `#[non_exhaustive]`), so the production `map_http` helper is split
//! into a pure `classify_http(ctx, status, code)` function that these
//! tests drive directly.

use zad::error::ZadError;
use zad::service::discord::client::{HttpCtx, classify_http};

#[test]
fn channel_404_maps_to_channel_not_found() {
    let err = classify_http(HttpCtx::Channel(123), 404, 0).expect("mapped");
    match err {
        ZadError::DiscordChannelNotFound { id } => assert_eq!(id, 123),
        other => panic!("expected DiscordChannelNotFound, got {other:?}"),
    }
}

#[test]
fn channel_discord_code_10003_maps_to_channel_not_found_even_on_403() {
    // Discord returns "Unknown Channel" (10003) with status 403 when the
    // bot can see the guild but not the specific channel. The typed
    // variant must still fire — the numeric code is the signal, not the
    // HTTP status.
    let err = classify_http(HttpCtx::Channel(456), 403, 10003).expect("mapped");
    match err {
        ZadError::DiscordChannelNotFound { id } => assert_eq!(id, 456),
        other => panic!("expected DiscordChannelNotFound, got {other:?}"),
    }
}

#[test]
fn dm_user_404_maps_to_channel_not_found_with_user_id() {
    // `--dm` paths carry the user snowflake into the error, not a
    // channel id. Users grepping logs for their target id should still
    // find it.
    let err = classify_http(HttpCtx::User(789), 404, 0).expect("mapped");
    match err {
        ZadError::DiscordChannelNotFound { id } => assert_eq!(id, 789),
        other => panic!("expected DiscordChannelNotFound, got {other:?}"),
    }
}

#[test]
fn guild_members_403_maps_to_privileged_intent() {
    let err = classify_http(HttpCtx::GuildMembers, 403, 0).expect("mapped");
    match err {
        ZadError::DiscordPrivilegedIntent { intent } => assert_eq!(intent, "GUILD_MEMBERS"),
        other => panic!("expected DiscordPrivilegedIntent, got {other:?}"),
    }
}

#[test]
fn channel_500_falls_through_to_generic() {
    // Non-404, non-10003 errors on channel calls should NOT be mapped
    // to the typed variant — the caller's existing `to_string` fallback
    // path is the right place for them.
    assert!(classify_http(HttpCtx::Channel(1), 500, 0).is_none());
}

#[test]
fn guild_members_500_falls_through_to_generic() {
    assert!(classify_http(HttpCtx::GuildMembers, 500, 0).is_none());
}
