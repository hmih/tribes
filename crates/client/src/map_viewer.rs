//! Map viewer plugin: loads the assembled glTF scene + terrain for a map and
//! spawns gameplay actor gizmos (spawns, flag bases, stations, turrets) from
//! the `.scene.actors.json` file.
//!
//! Three assets are loaded per [`MapLoadRequest`]:
//! - `<Map>.gltf` — combined static-mesh scene (already in glTF space).
//! - `<Map>_Ter.terrain.gltf` — terrain mesh (already in glTF space). May be absent; load failures
//!   are silently ignored.
//! - `<Map>.scene.actors.json` — gameplay actor data (UE3 space, converted at spawn time).
//!
//! Gameplay actors are spawned as marker entities carrying
//! [`GameplayMarker`] and [`Team`](tribes_core::Team) components. A gizmo
//! system draws colored spheres/boxes at their locations every frame.

use std::f32::consts::FRAC_PI_2;

use bevy::asset::LoadState;
use bevy::prelude::{DefaultGizmoConfigGroup, GizmoLineConfig, GizmoLineJoint, *};
use tribes_assets::{AssetsPlugin, MapActors, t3d};
use tribes_core::Team;

/// Resource prefix (relative to `AssetServer` root) for the assets directory.
///
/// Default assumes the `bin/client` is run with the `AssetServer` folder
/// pointed at `src/decompile/assets/`. Override via [`MapViewerPlugin::new`].
const DEFAULT_ASSETS_ROOT: &str = "../../decompile/assets";

/// Message requesting a map load. The string is the map directory name
/// (e.g. `"Perdition"`); files are resolved as `maps/<Map>/<Map>.gltf`,
/// `maps/<Map>/<Map>_Ter.terrain.gltf`, and `maps/<Map>/<Map>.scene.actors.json`.
#[derive(Message, Clone, Debug)]
pub struct MapLoadRequest(pub String);

/// Tracks handles for the currently-loading map. The static scene, terrain
/// scene, and gameplay actor markers are each spawned once, in their own
/// systems, the frame after the corresponding asset finishes loading.
#[derive(Resource, Default)]
struct PendingMap {
    gltf: Handle<bevy::gltf::Gltf>,
    terrain: Option<Handle<bevy::gltf::Gltf>>,
    actors: Handle<MapActors>,
    spawned_static: bool,
    spawned_terrain: bool,
    spawned_actors: bool,
    map_name: String,
}

#[derive(Resource)]
struct MapAssetsRoot(String);

/// Marker on spawned gameplay actor entities, carrying their kind + team.
#[derive(Component, Debug)]
pub struct GameplayMarker {
    pub kind: MarkerKind,
    pub team: Team,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerKind {
    PlayerSpawn,
    FlagBase,
    InventoryStation,
    RepairStation,
    BaseTurret,
    PowerGenerator,
    RadarStation,
    Other,
}

impl MarkerKind {
    fn from_class(class: &str) -> Self {
        if class == "UTTeamPlayerStart" || class == "PlayerStart" {
            Self::PlayerSpawn
        } else if class.starts_with("TrCTFBase_") {
            Self::FlagBase
        } else if class.starts_with("TrInventoryStation_") {
            Self::InventoryStation
        } else if class.starts_with("TrRepairStation_") {
            Self::RepairStation
        } else if class.starts_with("TrBaseTurret_") {
            Self::BaseTurret
        } else if class.starts_with("TrPowerGenerator_") {
            Self::PowerGenerator
        } else if class.starts_with("TrRadarStation_") {
            Self::RadarStation
        } else {
            Self::Other
        }
    }
}

pub struct MapViewerPlugin {
    assets_root: String,
}

impl Default for MapViewerPlugin {
    fn default() -> Self {
        Self {
            assets_root: DEFAULT_ASSETS_ROOT.to_string(),
        }
    }
}

impl MapViewerPlugin {
    pub fn new(assets_root: impl Into<String>) -> Self {
        Self {
            assets_root: assets_root.into(),
        }
    }
}

impl Plugin for MapViewerPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(AssetsPlugin)
            .add_message::<MapLoadRequest>()
            .insert_resource(PendingMap::default())
            .insert_resource(MapAssetsRoot(self.assets_root.clone()))
            .insert_gizmo_config::<DefaultGizmoConfigGroup>(
                DefaultGizmoConfigGroup,
                GizmoConfig {
                    line: GizmoLineConfig {
                        joints: GizmoLineJoint::Round(4),
                        ..default()
                    },
                    ..default()
                },
            )
            .add_systems(
                Update,
                (
                    handle_map_load_request,
                    spawn_static_scene,
                    spawn_terrain_scene,
                    spawn_actor_markers,
                    draw_gameplay_marker_gizmos,
                )
                    .chain(),
            );
    }
}

fn handle_map_load_request(
    mut reader: MessageReader<MapLoadRequest>,
    mut pending: ResMut<PendingMap>,
    asset_server: Res<AssetServer>,
    assets_root: Res<MapAssetsRoot>,
) {
    for request in reader.read() {
        let map = &request.0;
        let map_dir = format!("{}/maps/{}", assets_root.0, map);
        let gltf = asset_server.load(format!("{map_dir}/{map}.gltf"));
        let terrain = asset_server.load(format!("{map_dir}/{map}_Ter.terrain.gltf"));
        let actors = asset_server.load(format!("{map_dir}/{map}.scene.actors.json"));

        info!(map = %map, "Loading map assets");
        *pending = PendingMap {
            gltf,
            terrain: Some(terrain),
            actors,
            spawned_static: false,
            spawned_terrain: false,
            spawned_actors: false,
            map_name: map.clone(),
        };
    }
}

fn spawn_static_scene(
    mut pending: ResMut<PendingMap>,
    gltf_assets: Res<Assets<bevy::gltf::Gltf>>,
    asset_server: Res<AssetServer>,
    mut commands: Commands,
) {
    if pending.spawned_static || pending.map_name.is_empty() {
        return;
    }
    if !asset_server.is_loaded(pending.gltf.id()) {
        return;
    }
    let Some(gltf) = gltf_assets.get(&pending.gltf) else {
        return;
    };
    let Some(scene) = gltf.scenes.first() else {
        return;
    };
    commands.spawn((
        WorldAssetRoot(scene.clone()),
        Name::new(format!("Map: {}", pending.map_name)),
    ));
    pending.spawned_static = true;
    info!(map = %pending.map_name, "Static-mesh scene spawned");
}

fn spawn_terrain_scene(
    mut pending: ResMut<PendingMap>,
    gltf_assets: Res<Assets<bevy::gltf::Gltf>>,
    asset_server: Res<AssetServer>,
    mut commands: Commands,
) {
    if pending.spawned_terrain || pending.map_name.is_empty() {
        return;
    }
    let Some(terrain_handle) = pending.terrain.as_ref() else {
        pending.spawned_terrain = true;
        return;
    };
    let id = terrain_handle.id();
    if !asset_server.is_loaded(id) {
        return;
    }
    // Tolerate missing terrain files (Arena has none). If the asset failed to
    // load, mark as done without spawning.
    if let Some(state) = asset_server.get_load_state(id)
        && matches!(state, LoadState::Failed(_))
    {
        pending.spawned_terrain = true;
        debug!(map = %pending.map_name, "No terrain glTF for this map");
        return;
    }
    let Some(gltf) = gltf_assets.get(terrain_handle) else {
        return;
    };
    let Some(scene) = gltf.scenes.first() else {
        pending.spawned_terrain = true;
        return;
    };
    commands.spawn((
        WorldAssetRoot(scene.clone()),
        Name::new(format!("Terrain: {}", pending.map_name)),
    ));
    pending.spawned_terrain = true;
    info!(map = %pending.map_name, "Terrain scene spawned");
}

fn spawn_actor_markers(
    mut pending: ResMut<PendingMap>,
    actor_assets: Res<Assets<MapActors>>,
    asset_server: Res<AssetServer>,
    mut commands: Commands,
) {
    if pending.spawned_actors || pending.map_name.is_empty() {
        return;
    }
    if !asset_server.is_loaded(pending.actors.id()) {
        return;
    }
    let Some(actors) = actor_assets.get(&pending.actors) else {
        return;
    };

    // UE3→glTF axis transform: matches the build-time gltf_assembler.
    let c = Quat::from_rotation_x(FRAC_PI_2);

    info!(
        count = actors.actors.len(),
        "Spawning gameplay actor markers"
    );
    for actor in &actors.actors {
        let kind = MarkerKind::from_class(&actor.class);
        if matches!(kind, MarkerKind::Other) {
            continue;
        }
        let team = Team::from_class(&actor.class).unwrap_or_else(|| {
            if let Some(text) = actor.properties.get("TeamIndex") {
                Team::from_index(t3d::int(text, "TeamIndex"))
            } else if let Some(text) = actor.properties.get("TeamNumber") {
                Team::from_index(t3d::int(text, "TeamNumber"))
            } else {
                Team::BloodEagle
            }
        });

        let pos_ue = Vec3::from_array(actor.location);
        let pos_gltf = Vec3::new(pos_ue.x, pos_ue.z, -pos_ue.y);
        let q_ue = Quat::from_array(actor.rotation);
        let q_gltf = c * q_ue * c.inverse();

        commands.spawn((
            GameplayMarker { kind, team },
            Transform::from_translation(pos_gltf).with_rotation(q_gltf),
            GlobalTransform::IDENTITY,
            Visibility::default(),
            Name::new(format!("{} ({:?})", actor.class, team)),
        ));
    }
    pending.spawned_actors = true;
    info!("Actor markers spawned");
}

fn draw_gameplay_marker_gizmos(
    query: Query<(&GameplayMarker, &GlobalTransform)>,
    mut gizmos: Gizmos,
) {
    for (marker, tf) in &query {
        let pos = tf.translation();
        let color = match marker.team {
            Team::BloodEagle => Color::srgb(0.95, 0.25, 0.15),
            Team::DiamondSword => Color::srgb(0.2, 0.55, 0.95),
        };
        let iso = Isometry3d::from_translation(pos);
        match marker.kind {
            MarkerKind::PlayerSpawn => {
                gizmos.sphere(iso, 120.0, color);
                let fwd = tf.rotation() * Vec3::NEG_Z;
                gizmos.line(pos, pos + fwd * 400.0, color);
            }
            MarkerKind::FlagBase => {
                gizmos.cube(
                    Transform::from_translation(pos).with_scale(Vec3::splat(400.0)),
                    color,
                );
            }
            MarkerKind::InventoryStation
            | MarkerKind::RepairStation
            | MarkerKind::BaseTurret
            | MarkerKind::PowerGenerator
            | MarkerKind::RadarStation => {
                gizmos.sphere(iso, 180.0, color);
            }
            MarkerKind::Other => {
                gizmos.sphere(iso, 80.0, Color::srgb(0.8, 0.8, 0.8));
            }
        }
    }
}
