//! The Pane runtime. Owns the core thread that runs the reducer, the
//! per-app workers, persistence and the control socket. Used by the
//! headless `pane-agentd` and, through `pane-ffi`, by `Pane.app`.

pub mod control;
pub mod lockfile;
pub mod logging;
pub mod paths;
pub mod persist;
pub mod platform;
pub mod runtime;

pub use runtime::{Handle, Options};
