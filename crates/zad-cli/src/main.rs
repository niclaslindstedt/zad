use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match zad_cli::cli::run().await {
        Ok(()) if zad_cli::cli::echo::was_echoed() => ExitCode::from(3),
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // Rate-limit errors are the one shape we surface as
            // structured JSON when the caller asked for machine
            // output. Every other error continues to render through
            // the human stderr path; --json's success contract on
            // each subcommand is unchanged.
            if let zad::ZadError::RateLimited {
                service,
                retry_after_seconds,
                retry_after_utc,
            } = &e
                && json_output_requested()
            {
                let payload = serde_json::json!({
                    "error": "rate_limited",
                    "service": service,
                    "retry_after_seconds": retry_after_seconds,
                    "retry_after_utc": retry_after_utc,
                    "message": e.to_string(),
                    "hint": "re-run the same command with --wait to block until ready and retry automatically",
                });
                println!("{payload}");
            } else {
                zad_cli::output::error(&e.to_string());
            }
            ExitCode::from(1)
        }
    }
}

/// Did the user pass a `--json` flag anywhere on the command line?
/// `--json` is a per-subcommand flag rather than a global one, so we
/// inspect argv directly instead of plumbing it through every command
/// handler. We treat any of `--json`, `--json=true`, or
/// `--json true` as opting in.
fn json_output_requested() -> bool {
    std::env::args()
        .skip(1)
        .any(|a| a == "--json" || a.starts_with("--json="))
}
