use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match zad::cli::run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            tracing::error!(%e, "command failed");
            ExitCode::from(1)
        }
    }
}
