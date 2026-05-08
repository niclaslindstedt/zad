use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match zad::cli::run().await {
        Ok(()) if zad::cli::echo::was_echoed() => ExitCode::from(3),
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            zad::output::error(&e.to_string());
            ExitCode::from(1)
        }
    }
}
