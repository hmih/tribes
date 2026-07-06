//! Client-side Bevy plugins: rendering, prediction, audio, UI.

pub mod flycam;
pub mod map_viewer;

pub use flycam::FlycamPlugin;
pub use map_viewer::{MapLoadRequest, MapViewerPlugin};
