//! macOS side of Pane. Everything here is `cfg(target_os = "macos")`; on
//! other platforms the crate is empty so the workspace still builds.

#[cfg(target_os = "macos")]
pub mod apps;
#[cfg(target_os = "macos")]
pub mod ax;
#[cfg(target_os = "macos")]
pub mod display;
#[cfg(target_os = "macos")]
pub mod ffi;
#[cfg(target_os = "macos")]
pub mod hotkeys;
#[cfg(target_os = "macos")]
pub mod observer;
#[cfg(target_os = "macos")]
pub mod pointer;
#[cfg(target_os = "macos")]
pub mod recovery;
#[cfg(target_os = "macos")]
pub mod runloop;
#[cfg(target_os = "macos")]
pub mod touches;
#[cfg(target_os = "macos")]
pub mod window_probe;
#[cfg(target_os = "macos")]
pub mod worker;
