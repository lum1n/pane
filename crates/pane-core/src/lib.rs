//! Pure model of the Pane window manager.
//!
//! Nothing in this crate talks to macOS. The runtime (`pane-agent`) feeds
//! [`event::Event`]s into [`reducer::reduce`], runs [`reconcile::reconcile`]
//! and carries out the returned [`effect::Effect`]s. Everything here is
//! deterministic and unit-tested on any platform.

pub mod assign;
pub mod config;
pub mod display;
pub mod effect;
pub mod event;
pub mod geometry;
pub mod identity;
pub mod layout;
pub mod model;
pub mod profile;
pub mod proto;
pub mod reconcile;
pub mod reducer;
pub mod rules;
pub mod snapshot;
pub mod state;
pub mod tree;
pub mod worker;
pub mod workspace;

pub use geometry::{Direction, Orientation, Point, Rect, Size};

/// CGWindowID of a window.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct WindowId(pub u32);

impl std::fmt::Display for WindowId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Process id of an application.
pub type Pid = i32;

/// Workspace name ("1", "web", ...).
pub type WorkspaceId = smol_str::SmolStr;

/// Stable display identifier: the CGDisplay UUID string.
pub type DisplayId = String;
