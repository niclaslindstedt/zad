use std::path::PathBuf;
use std::sync::OnceLock;

use directories::ProjectDirs;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

static GUARD: OnceLock<WorkerGuard> = OnceLock::new();

pub fn log_dir() -> Option<PathBuf> {
    let dirs = ProjectDirs::from("", "", "zad")?;
    Some(dirs.state_dir().unwrap_or(dirs.data_local_dir()).to_owned())
}

pub fn log_path() -> Option<PathBuf> {
    log_dir().map(|d| d.join("debug.log"))
}

pub fn init(debug: bool) {
    let default = if debug {
        "zad=debug,info"
    } else {
        "zad=info,warn"
    };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default));

    let stderr_layer = fmt::layer().with_writer(std::io::stderr).with_target(false);

    let subscriber = tracing_subscriber::registry()
        .with(filter)
        .with(stderr_layer);

    if let Some(dir) = log_dir()
        && std::fs::create_dir_all(&dir).is_ok()
    {
        let appender = tracing_appender::rolling::daily(&dir, "debug.log");
        let (nb, guard) = tracing_appender::non_blocking(appender);
        let _ = GUARD.set(guard);
        let file_layer = fmt::layer()
            .with_writer(nb)
            .with_ansi(false)
            .with_target(true);
        let _ = subscriber.with(file_layer).try_init();
        return;
    }
    let _ = subscriber.try_init();
}
