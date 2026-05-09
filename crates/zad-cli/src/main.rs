use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match zad_cli::cli::run().await {
        Ok(()) if zad_cli::cli::echo::was_echoed() => ExitCode::from(3),
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            zad_cli::output::error(&e.to_string());
            ExitCode::from(1)
        }
    }
}
