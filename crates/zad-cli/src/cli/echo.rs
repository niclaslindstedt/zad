//! Echo-mode for runtime verbs whose permissions file can't be trusted.
//!
//! When `permissions::signing::verify_raw` rejects a permissions file
//! (no trust entry, tampered bytes, rotated key, broken trust store, or
//! missing keychain), runtime verbs should NOT execute their network
//! call — but they also shouldn't just print an opaque error and exit.
//! The operator wants to see *what would have been issued*, plus the
//! reason, so they can iterate on permission files without round-tripping
//! through real API calls.
//!
//! ## Flow
//!
//! 1. The verb calls `<service>::permissions::load_effective_or_echo()`
//!    instead of `load_effective()`.
//! 2. If the load fails with a [signing error][zad::permissions::signing::is_signing_error],
//!    the helper [`arm`]s this module with an [`EchoReason`] and returns
//!    a permissive [`EffectivePermissions::default()`] so subsequent
//!    `check_*` calls on the verb's hot path no-op.
//! 3. The verb's transport selector (`discord_http_for` etc.) checks
//!    [`echo_active`]; when it is on, it returns the existing
//!    `DryRun*Transport` wired to [`dry_run_sink_for_echo`] (a buffer
//!    instead of stderr/stdout).
//! 4. After the verb's transport call returns, the verb invokes
//!    [`render_and_clear`], which drains the captured [`DryRunOp`]s,
//!    pairs them with the [`EchoReason`], prints either a human-readable
//!    summary or a structured JSON envelope, and calls [`mark_echoed`].
//! 5. `main.rs` reads [`was_echoed`] after the verb returns and exits
//!    with code `3` instead of `0` so callers can distinguish "ran" from
//!    "echoed" from "failed".
//!
//! Diagnostic verbs (`permissions show|check|path|init|...`) keep
//! calling `load_effective()` directly so signing errors surface there
//! — that's the surface the operator uses to *fix* a broken trust
//! state, and silently echoing them would obscure the failure mode.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;
use serde_json::json;

use zad::error::{Result, ZadError};
use zad::permissions::signing;
use zad::service::{DryRunOp, DryRunSink};

/// Why the echo path was taken. Mirrors the five signing-related
/// `ZadError` variants. `kind` is a stable string tag callers may
/// switch on; `reason` is the user-facing message; `path` is the
/// permissions (or trust store) file the operator should fix.
#[derive(Debug, Clone, Serialize)]
pub struct EchoReason {
    pub kind: &'static str,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// Build an [`EchoReason`] from a [`ZadError`]. Returns `None` for
/// errors that are *not* signing-related — those should keep their
/// hard-fail shape.
pub fn from_signing_error(err: &ZadError) -> Option<EchoReason> {
    match err {
        ZadError::NotTrusted { path, .. } => Some(EchoReason {
            kind: "not_trusted",
            reason: err.to_string(),
            path: Some(path.display().to_string()),
        }),
        ZadError::SignatureInvalid { path, .. } => Some(EchoReason {
            kind: "signature_invalid",
            reason: err.to_string(),
            path: Some(path.display().to_string()),
        }),
        ZadError::SignatureKeyMismatch { path, .. } => Some(EchoReason {
            kind: "signature_key_mismatch",
            reason: err.to_string(),
            path: Some(path.display().to_string()),
        }),
        ZadError::TrustStoreTampered { path, .. } => Some(EchoReason {
            kind: "trust_store_tampered",
            reason: err.to_string(),
            path: Some(path.display().to_string()),
        }),
        ZadError::SigningKeyMissing { .. } => Some(EchoReason {
            kind: "signing_key_missing",
            reason: err.to_string(),
            path: None,
        }),
        _ => None,
    }
}

static ECHO_STATE: OnceLock<Mutex<Option<EchoReason>>> = OnceLock::new();
static ECHO_SINK: OnceLock<Arc<EchoSink>> = OnceLock::new();
static ECHOED: AtomicBool = AtomicBool::new(false);

fn state() -> &'static Mutex<Option<EchoReason>> {
    ECHO_STATE.get_or_init(|| Mutex::new(None))
}

fn shared_sink() -> &'static Arc<EchoSink> {
    ECHO_SINK.get_or_init(|| {
        Arc::new(EchoSink {
            buf: Mutex::new(Vec::new()),
        })
    })
}

/// Buffer the next captured [`DryRunOp`] from a transport instead of
/// printing it. Replaces [`zad::service::default_dry_run_sink`] when
/// echo mode is active so the verb-end can render op + reason together.
pub struct EchoSink {
    buf: Mutex<Vec<DryRunOp>>,
}

impl DryRunSink for EchoSink {
    fn record(&self, op: DryRunOp) {
        self.buf.lock().expect("echo sink poisoned").push(op);
    }
}

/// Stash an [`EchoReason`] for the current invocation. Called by
/// [`load_effective_or_echo`] when `verify_raw` rejects the file.
pub fn arm(reason: EchoReason) {
    *state().lock().expect("echo state poisoned") = Some(reason);
}

/// Wrap a per-service `load_effective` call with echo-mode arming.
/// On a signing error (untrusted file, tampered bytes, rotated key,
/// broken trust store, missing keychain), [`arm`] the reason and
/// return a permissive [`Default`] permissions value so the verb's
/// `check_*` calls no-op and the verb's transport selector can switch
/// to the buffered dry-run path.
///
/// Non-signing errors propagate unchanged. Diagnostic verbs
/// (`permissions show|check|...`) bypass this wrapper and call
/// `load_effective` directly so signing errors surface there.
pub fn load_effective_or_echo<P, F>(loader: F) -> Result<P>
where
    P: Default,
    F: FnOnce() -> Result<P>,
{
    match loader() {
        Ok(p) => Ok(p),
        Err(e) if signing::is_signing_error(&e) => {
            if let Some(reason) = from_signing_error(&e) {
                arm(reason);
            }
            Ok(P::default())
        }
        Err(e) => Err(e),
    }
}

/// `true` once an [`EchoReason`] has been armed and not yet rendered.
/// Transport selectors switch to the dry-run + buffered sink path while
/// this is on.
pub fn echo_active() -> bool {
    state().lock().expect("echo state poisoned").is_some()
}

/// Sink to hand to a `DryRun*Transport` when [`echo_active`] is true.
/// All transports share one buffer so [`render_and_clear`] can drain it
/// regardless of which verb captured the op.
pub fn dry_run_sink_for_echo() -> Arc<dyn DryRunSink> {
    shared_sink().clone()
}

/// Set the process-global "this run echoed" flag. `main.rs` reads it
/// to pick exit code 3 over 0.
pub fn mark_echoed() {
    ECHOED.store(true, Ordering::SeqCst);
}

/// `true` if any verb called [`mark_echoed`] this invocation.
pub fn was_echoed() -> bool {
    ECHOED.load(Ordering::SeqCst)
}

/// Reset all echo state. Library tests that exercise multiple
/// invocations in one process call this between runs; the binary never
/// needs it (one CLI invocation = one process).
#[doc(hidden)]
pub fn reset_for_test() {
    *state().lock().expect("echo state poisoned") = None;
    shared_sink()
        .buf
        .lock()
        .expect("echo sink poisoned")
        .clear();
    ECHOED.store(false, Ordering::SeqCst);
}

#[derive(Debug, Serialize)]
struct EchoEnvelope<'a> {
    /// Structured payload of the call that would have been issued.
    /// `null` when the verb didn't call any mutating transport method
    /// (read-only verbs return early without recording).
    echoed: &'a serde_json::Value,
    error: &'a EchoReason,
}

/// Drain the captured op + armed reason and render them to stdout in
/// the requested format, then call [`mark_echoed`].
///
/// Verbs invoke this in place of their normal success print at the
/// point where they would otherwise have exited with `Ok(())`. If no
/// reason is armed (i.e. echo mode wasn't actually triggered), this is
/// a no-op — the verb's caller still prints its real success output
/// because [`mark_echoed`] is never called.
pub fn render_and_clear(json: bool) {
    let Some(reason) = state().lock().expect("echo state poisoned").take() else {
        return;
    };
    let ops: Vec<DryRunOp> = {
        let mut buf = shared_sink().buf.lock().expect("echo sink poisoned");
        buf.drain(..).collect()
    };

    if json {
        let payload = match ops.first() {
            Some(op) => serde_json::to_value(EchoEnvelope {
                echoed: &op.details,
                error: &reason,
            })
            .unwrap_or_else(|_| json!({ "error": &reason })),
            None => json!({
                "echoed": null,
                "error": &reason,
            }),
        };
        match serde_json::to_string_pretty(&payload) {
            Ok(rendered) => println!("{rendered}"),
            Err(e) => eprintln!("echo: failed to render payload as JSON: {e}"),
        }
    } else {
        if ops.is_empty() {
            println!("would have run: (no transport call captured)");
        } else {
            for op in &ops {
                println!("would have run: {}", op.summary);
            }
        }
        println!("  reason: {}", reason.reason);
    }
    mark_echoed();
}
