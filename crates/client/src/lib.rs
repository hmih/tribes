//! Client-side Bevy plugins: rendering, prediction, audio, UI.

pub mod flycam;
pub mod map_viewer;
pub mod shot;

pub use flycam::FlycamPlugin;
pub use map_viewer::{MapLoadRequest, MapViewerPlugin};
pub use shot::DiagnosticShotPlugin;
