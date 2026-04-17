use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match zad::cli::run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            zad::output::error(&e.to_string());
            ExitCode::from(1)
        }
    }
}
