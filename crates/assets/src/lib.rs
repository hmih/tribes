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

use std::collections::HashMap;
use std::fmt;

use bevy::app::Plugin;
use bevy::asset::Asset;
use bevy::prelude::AssetApp;
use bevy::reflect::TypePath;
use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

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

/// A single property from a UELib `.actors.json` object.
///
/// Properties are emitted as `{name, type, size, array_index, value}` where
/// `value` is T3D-format text (e.g. `"TeamIndex=1"`).
#[derive(Debug, Clone, Deserialize)]
struct RawProperty {
    name: String,
    value: String,
}

/// A gameplay actor (spawn point, flag base, turret, generator, volume, ...),
/// in **UE3 coordinates**.
///
/// See the module docs for the UE3→glTF conversion formulas. The `properties`
/// field is a map of property name → raw T3D text value. Use [`t3d::scalar`]
/// for simple typed extraction.
#[derive(Debug, Clone, Serialize, TypePath, Asset)]
pub struct GameplayActor {
    pub name: String,
    pub class: String,
    pub location: [f32; 3],
    /// UE3-space quaternion `(x, y, z, w)`.
    pub rotation: [f32; 4],
    pub properties: HashMap<String, String>,
}

fn parse_ue3_vec3(text: &str) -> Option<[f32; 3]> {
    // Format: "Location=(X=105826.9531250,Y=-16284.0644531,Z=0.0000000)"
    let inner = text.strip_prefix("Location=(")?.strip_suffix(')')?;
    let mut x = None;
    let mut y = None;
    let mut z = None;
    for part in inner.split(',') {
        let (key, val) = part.split_once('=')?;
        match key.trim() {
            "X" => x = val.trim().parse().ok(),
            "Y" => y = val.trim().parse().ok(),
            "Z" => z = val.trim().parse().ok(),
            _ => {}
        }
    }
    Some([x?, y?, z?])
}

fn ue3_rotator_to_quat(pitch: i32, yaw: i32, roll: i32) -> [f32; 4] {
    const UNITS: f64 = 65536.0;
    const TAU: f64 = std::f64::consts::TAU;

    let half_yaw = (yaw as f64 / UNITS * TAU) / 2.0;
    let half_pitch = (pitch as f64 / UNITS * TAU) / 2.0;
    let half_roll = (roll as f64 / UNITS * TAU) / 2.0;

    let (cy, sy) = (half_yaw.cos(), half_yaw.sin());
    let (cp, sp) = (half_pitch.cos(), half_pitch.sin());
    let (cr, sr) = (half_roll.cos(), half_roll.sin());

    // q_yaw (Z), q_pitch (Y), q_roll (X), applied Yaw→Pitch→Roll
    let (qx, qy, qz, qw) = (
        (sr * cp * cy - cr * sp * sy) as f32,
        (cr * sp * cy + sr * cp * sy) as f32,
        (cr * cp * sy - sr * sp * cy) as f32,
        (cr * cp * cy + sr * sp * sy) as f32,
    );
    [qx, qy, qz, qw]
}

fn parse_ue3_rotator(text: &str) -> Option<[f32; 4]> {
    // Format: "Rotation=(Pitch=0,Yaw=32768,Roll=0)"
    let inner = text.strip_prefix("Rotation=(")?.strip_suffix(')')?;
    let mut pitch = 0i32;
    let mut yaw = 0i32;
    let mut roll = 0i32;
    for part in inner.split(',') {
        let (key, val) = part.split_once('=')?;
        let v: i32 = val.trim().parse().ok()?;
        match key.trim() {
            "Pitch" => pitch = v,
            "Yaw" => yaw = v,
            "Roll" => roll = v,
            _ => {}
        }
    }
    Some(ue3_rotator_to_quat(pitch, yaw, roll))
}

impl<'de> Deserialize<'de> for GameplayActor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ActorVisitor;

        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "lowercase")]
        enum Field {
            Name,
            Class,
            Outer,
            Properties,
        }

        impl<'de> Visitor<'de> for ActorVisitor {
            type Value = GameplayActor;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a UELib actor object")
            }

            fn visit_map<M>(self, mut map: M) -> Result<GameplayActor, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut name = String::new();
                let mut class = String::new();
                let mut props: Vec<RawProperty> = Vec::new();

                while let Some(key) = map.next_key::<Field>()? {
                    match key {
                        Field::Name => name = map.next_value()?,
                        Field::Class => class = map.next_value()?,
                        Field::Outer => {
                            let _: Option<String> = map.next_value()?;
                        }
                        Field::Properties => props = map.next_value()?,
                    }
                }

                let mut properties: HashMap<String, String> = HashMap::with_capacity(props.len());
                let mut location: [f32; 3] = [0.0, 0.0, 0.0];
                let mut rotation: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

                for p in props {
                    if p.name == "Location"
                        && let Some(loc) = parse_ue3_vec3(&p.value)
                    {
                        location = loc;
                        continue;
                    }
                    if p.name == "Rotation"
                        && let Some(rot) = parse_ue3_rotator(&p.value)
                    {
                        rotation = rot;
                        continue;
                    }
                    properties.insert(p.name, p.value);
                }

                Ok(GameplayActor {
                    name,
                    class,
                    location,
                    rotation,
                    properties,
                })
            }
        }

        deserializer.deserialize_struct(
            "GameplayActor",
            &["name", "class", "outer", "properties"],
            ActorVisitor,
        )
    }
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
///
/// Deserialized from UELib MapExtractor output. Field names are
/// `object_count` / `objects` in the JSON, mapped to Rust-friendly names.
#[derive(Debug, Clone, Deserialize, Serialize, TypePath, Asset)]
pub struct MapActors {
    pub map: String,
    #[serde(rename = "object_count")]
    pub actor_count: usize,
    #[serde(rename = "objects")]
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
