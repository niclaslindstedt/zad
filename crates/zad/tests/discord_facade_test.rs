//! Typed-input contract for the Discord facade. These tests exercise
//! the validating constructors only — no network, no keychain, no
//! filesystem — so they double as compile-time evidence that the
//! library refuses malformed calls.

use std::path::PathBuf;

use zad::ZadError;
use zad::service::discord::client::{DISCORD_MAX_ATTACHMENTS, DISCORD_MAX_MESSAGE_LEN};
use zad::service::discord::{MessageBody, ReadRequest, SendRequest};
use zad::service::{ChannelId, Target};

fn channel(id: u64) -> Target {
    Target::Channel(ChannelId(id))
}

#[test]
fn send_request_accepts_a_normal_message() {
    let req = SendRequest::new(channel(123), MessageBody::text("hello"), vec![]);
    assert!(req.is_ok());
}

#[test]
fn send_request_rejects_oversized_body() {
    let body = "a".repeat(DISCORD_MAX_MESSAGE_LEN + 1);
    let err = SendRequest::new(channel(123), MessageBody::text(body), vec![])
        .expect_err("oversized body must be rejected at construction time");
    match err {
        ZadError::Invalid(msg) => assert!(
            msg.contains("hard limit"),
            "expected length-limit message, got: {msg}"
        ),
        other => panic!("expected ZadError::Invalid, got {other:?}"),
    }
}

#[test]
fn send_request_rejects_too_many_attachments() {
    let attachments: Vec<PathBuf> = (0..=DISCORD_MAX_ATTACHMENTS)
        .map(|i| PathBuf::from(format!("/tmp/file_{i}.png")))
        .collect();
    let err = SendRequest::new(channel(123), MessageBody::text("x"), attachments)
        .expect_err("too many attachments must be rejected at construction time");
    match err {
        ZadError::Invalid(msg) => assert!(
            msg.contains("per-message cap"),
            "expected attachment-cap message, got: {msg}"
        ),
        other => panic!("expected ZadError::Invalid, got {other:?}"),
    }
}

#[test]
fn send_request_rejects_empty_body_with_no_attachments() {
    let err = SendRequest::new(channel(123), MessageBody::Empty, vec![])
        .expect_err("empty body with no attachments must be rejected");
    match err {
        ZadError::Invalid(msg) => assert!(
            msg.contains("empty"),
            "expected empty-payload message, got: {msg}"
        ),
        other => panic!("expected ZadError::Invalid, got {other:?}"),
    }
}

#[test]
fn send_request_accepts_empty_body_with_attachment() {
    let req = SendRequest::new(
        channel(123),
        MessageBody::Empty,
        vec![PathBuf::from("/tmp/cat.png")],
    );
    assert!(req.is_ok());
}

#[test]
fn read_request_rejects_zero_limit() {
    let err = ReadRequest::new(ChannelId(7), 0).expect_err("zero limit must be rejected");
    match err {
        ZadError::Invalid(msg) => assert!(msg.contains("between 1 and 100"), "got: {msg}"),
        other => panic!("expected ZadError::Invalid, got {other:?}"),
    }
}

#[test]
fn read_request_rejects_over_one_hundred() {
    let err = ReadRequest::new(ChannelId(7), 101).expect_err("limit > 100 must be rejected");
    match err {
        ZadError::Invalid(msg) => assert!(msg.contains("between 1 and 100"), "got: {msg}"),
        other => panic!("expected ZadError::Invalid, got {other:?}"),
    }
}

#[test]
fn read_request_accepts_boundary_values() {
    assert!(ReadRequest::new(ChannelId(7), 1).is_ok());
    assert!(ReadRequest::new(ChannelId(7), 100).is_ok());
}
