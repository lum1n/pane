//! Tracing to `logs/pane.log` (daily rotation) and stderr.

use std::path::Path;
use std::sync::OnceLock;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

static GUARD: OnceLock<WorkerGuard> = OnceLock::new();

/// Set up logging once per process. `RUST_LOG` overrides the default `info`.
pub fn init(log_dir: &Path, also_stderr: bool) {
    if GUARD.get().is_some() {
        return;
    }
    let appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("pane")
        .filename_suffix("log")
        .max_log_files(7)
        .build(log_dir);
    let Ok(appender) = appender else { return };
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(writer)
        .with_ansi(false);
    let stderr_layer =
        also_stderr.then(|| tracing_subscriber::fmt::layer().with_writer(std::io::stderr));
    if tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(stderr_layer)
        .try_init()
        .is_ok()
    {
        let _ = GUARD.set(guard);
    }
}
