//! Asset loaders for converted Tribes map data.
//!
//! Two typed JSON assets are loaded at runtime via `bevy_common_assets::json`:
//! - [`MapScene`] (`.scene.json`): mesh instances + (legacy) actor list.
//! - [`MapActors`] (`.scene.actors.json`): gameplay actors (spawns, flag bases, turrets,
//!   generators) with class/location/rotation and a T3D text property map.
//!
//! ## Coordinate spaces
//!
//! Both JSON files store transforms in **UE3 space** (left-handed, Z-up):
//! - `location`: `(x, y, z)_ue` — straight from the decompiled map.
//! - `rotation`: quaternion derived from UE3 euler (Pitch/Yaw/Roll), still in UE3 axes.
//! - `scale3d`: `(sx, sy, sz)_ue`.
//!
//! The assembled `<Map>.gltf` and `<Map>_Ter.terrain.gltf` files are already
//! converted to **glTF space** (right-handed, Y-up) by the build-time
//! `gltf_assembler`. To place a JSON transform in Bevy's world, apply the
//! same UE3→glTF transform the assembler uses:
//! - position: `(x, y, z)_ue → (x, z, -y)_gltf`
//! - rotation: `q_gltf = C * q_ue * C⁻¹` where `C = Quat::from_rotation_x(π/2)`
//! - scale: `(sx, sy, sz)_ue → (sx, sz, sy)_gltf` (swap Y/Z, no negation)
//!
//! The `properties` field of [`GameplayActor`] is a map of property name → raw
//! T3D text value. Use [`t3d::scalar`] for simple typed extraction (int/float/
//! bool/string); full sub-object parsing is deferred to a later milestone.

pub mod t3d;

use bevy::app::Plugin;
use bevy::asset::Asset;
use bevy::prelude::AssetApp;
use bevy::reflect::TypePath;
use serde::{Deserialize, Serialize};

/// One static-mesh instance placed in the world, in **UE3 coordinates**.
///
/// See the module docs for the UE3→glTF conversion formulas.
#[derive(Debug, Clone, Deserialize, Serialize, TypePath, Asset)]
pub struct MeshInstance {
    pub actor_name: String,
    pub mesh_ref: String,
    pub location: [f32; 3],
    /// UE3-space quaternion `(x, y, z, w)`.
    pub rotation: [f32; 4],
    pub scale3d: [f32; 3],
    /// Multiplier applied on top of `scale3d` (UE3 `DrawScale`).
    pub draw_scale: f32,
}

impl MeshInstance {
    /// Returns the basename of this mesh in the flat `static-meshes/` dir
    /// (no extension).
    ///
    /// `Common_Ground.SM.SM_Icecoaster_SoftBorder` → `SM_Icecoaster_SoftBorder`.
    /// Returns `None` if `mesh_ref` has no dot or the trailing segment is empty.
    pub fn gltf_basename(&self) -> Option<&str> {
        let last = self.mesh_ref.rsplit('.').next()?;
        if last.is_empty() { None } else { Some(last) }
    }
}

/// A gameplay actor (spawn point, flag base, turret, generator, volume, ...),
/// in **UE3 coordinates**.
///
/// See the module docs for the UE3→glTF conversion formulas. The `properties`
/// field is a map of property name → raw T3D text value as emitted by the
/// decompiler. Values may be scalars (`"TeamIndex=1"`), struct literals
/// (`"NavGuid=(A=...,B=...,C=...,D=...)"`), object refs, or inline `begin object
/// ... end object` blocks. Use [`t3d::scalar`] for simple typed extraction.
#[derive(Debug, Clone, Deserialize, Serialize, TypePath, Asset)]
pub struct GameplayActor {
    pub name: String,
    pub class: String,
    pub location: [f32; 3],
    /// UE3-space quaternion `(x, y, z, w)`.
    pub rotation: [f32; 4],
    pub properties: std::collections::HashMap<String, String>,
}

/// Combined scene file (`.scene.json`): mesh instances + (legacy) actor list.
///
/// The actor list here is the same one also written to `.scene.actors.json`;
/// prefer [`MapActors`] for gameplay actor data to keep intent clear.
#[derive(Debug, Clone, Deserialize, Serialize, TypePath, Asset)]
pub struct MapScene {
    pub map_name: String,
    pub mesh_instances: Vec<MeshInstance>,
    /// Legacy field; populated identically to `MapActors::actors`. Ignored at
    /// runtime in favor of the dedicated `.scene.actors.json` loader.
    #[serde(default)]
    pub actors: Vec<GameplayActor>,
    /// Unique `mesh_ref` values referenced by `mesh_instances`. Useful for
    /// pre-warming the asset cache; not required for correctness.
    #[serde(default)]
    pub mesh_requirements: Vec<String>,
}

/// Dedicated gameplay-actor file (`.scene.actors.json`).
#[derive(Debug, Clone, Deserialize, Serialize, TypePath, Asset)]
pub struct MapActors {
    pub map: String,
    pub actor_count: usize,
    pub actors: Vec<GameplayActor>,
}

/// Bevy plugin registering the typed JSON asset loaders for map data.
///
/// After `add_plugins(AssetsPlugin)`, load assets via:
/// ```ignore
/// let scene: Handle<MapScene> = asset_server.load("maps/Perdition/Perdition.scene.json");
/// let actors: Handle<MapActors> = asset_server.load("maps/Perdition/Perdition.scene.actors.json");
/// ```
pub struct AssetsPlugin;

impl Plugin for AssetsPlugin {
    fn build(&self, app: &mut bevy::app::App) {
        app.init_asset::<MapScene>()
            .init_asset::<MapActors>()
            .add_plugins(bevy_common_assets::json::JsonAssetPlugin::<MapScene>::new(
                &["scene.json"],
            ))
            .add_plugins(bevy_common_assets::json::JsonAssetPlugin::<MapActors>::new(
                &["scene.actors.json"],
            ));
    }
}
