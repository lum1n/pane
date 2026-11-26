//! Headless Pane: everything except the menu bar UI.

use std::sync::Arc;
use std::time::Duration;

use pane_agent::paths::Paths;
use pane_agent::platform::native;
use pane_agent::{logging, runtime, Options};

fn main() {
    let paths = Paths::new(Paths::default_root());
    let _ = paths.ensure();
    logging::init(&paths.logs(), true);
    let platform: Arc<dyn pane_agent::platform::Platform> = Arc::from(native());
    let handle = match runtime::start(platform, Options::new(paths)) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("pane-agentd: {e}");
            std::process::exit(1);
        }
    };
    tracing::info!("pane-agentd running");

    // Clean exit on SIGTERM / SIGINT / SIGHUP: unpark everything first.
    let h = handle.clone();
    let mut signals = signal_hook::iterator::Signals::new([
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGHUP,
    ])
    .expect("signal handler");
    std::thread::spawn(move || {
        if let Some(sig) = signals.forever().next() {
            tracing::info!(sig, "signal received");
            h.shutdown(Duration::from_secs(3));
            std::process::exit(0);
        }
    });

    #[cfg(target_os = "macos")]
    {
        // Display changes are delivered on the main run loop.
        let h = handle.clone();
        pane_macos::display::watch(move || h.screens_changed());
        pane_macos::runloop::run_current();
    }
    #[cfg(not(target_os = "macos"))]
    loop {
        std::thread::sleep(Duration::from_secs(3600));
        let _ = &handle;
    }
}
