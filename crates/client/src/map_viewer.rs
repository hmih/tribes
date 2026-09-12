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

use bevy::asset::LoadState;
use bevy::camera::primitives::Aabb;
use bevy::light::GlobalAmbientLight;
use bevy::prelude::{DefaultGizmoConfigGroup, GizmoLineConfig, GizmoLineJoint, *};
use tribes_assets::{AssetsPlugin, MapActors, MapProps, t3d};
use tribes_core::Team;

/// Convert a UE3-space transform to glTF world space.
///
/// `(x, y, z)_ue → (x, z, y)_gltf`, with the quaternion's Y/Z components swapped
/// to match. This mirrors the map's `ue3_to_gltf_root` node matrix so props and
/// markers land on top of the static geometry.
///
/// The mapping is a reflection (det = -1), which is required rather than
/// accidental: UE3 is left-handed and glTF is right-handed, so a faithful
/// conversion is improper. Substituting a pure rotation mirrors the world (it
/// swaps the BloodEagle and DiamondSword bases). Because the scene is mirrored,
/// the assembler compensates triangle winding — do not "fix" that by making
/// materials double-sided, since Bevy inverts the normal on the back face of a
/// double-sided material and every up-facing surface goes dark.
fn ue3_to_gltf(location: &[f32; 3], rotation: &[f32; 4]) -> (Vec3, Quat) {
    (
        Vec3::new(location[0], location[2], location[1]),
        Quat::from_xyzw(rotation[0], rotation[2], rotation[1], rotation[3]),
    )
}

/// Default world-space size threshold (in glTF units) above which a mesh is
/// considered a sky-dome / occlusion hull and hidden from rendering. Perdition
/// has 4–5 such meshes spanning ~880k units; the actual map geometry is
/// bounded at ~210k units across. Tune via [`MapCullConfig`].
pub const DEFAULT_SKYDOME_CULL_SIZE: f32 = 5_000_000.0;

/// Resource prefix (relative to `AssetServer` root) for the assets directory.
///
/// Default assumes the `bin/client` is run with the `AssetServer` folder
/// pointed at `src/decompile/assets/`. Override via [`MapViewerPlugin::new`].
const DEFAULT_ASSETS_ROOT: &str = "gltf";

/// Message requesting a map load. The string is the map directory name
/// (e.g. `"Perdition"`); files are resolved as `<root>/maps/<Map>/<Map>.gltf`,
/// `<root>/maps/<Map>/<Map>_Ter.terrain.gltf`, and `<root>/maps/<Map>/<Map>.scene.actors.json`.
/// Default root is `gltf` (relative to Bevy asset `file_path`).
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
    props: Handle<MapProps>,
    spawned_static: bool,
    spawned_terrain: bool,
    spawned_actors: bool,
    spawned_props: bool,
    map_name: String,
}

#[derive(Resource)]
struct MapAssetsRoot(String);

/// Configures culling for loaded map meshes.
///
/// - `max_mesh_size`: meshes whose world-space AABB exceeds this on any axis are hidden (skydomes /
///   occlusion hulls). Set to `f32::INFINITY` to disable.
#[derive(Resource, Clone)]
pub struct MapCullConfig {
    pub max_mesh_size: f32,
}

impl Default for MapCullConfig {
    fn default() -> Self {
        Self {
            max_mesh_size: DEFAULT_SKYDOME_CULL_SIZE,
        }
    }
}

/// Marker added after a mesh has had its default material assigned. Lets the
/// per-frame system skip entities it has already touched.
#[derive(Component)]
struct DefaultMaterialApplied;

type UnmattedMeshQuery<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static Aabb,
        &'static GlobalTransform,
        Option<&'static MeshMaterial3d<StandardMaterial>>,
    ),
    (With<Mesh3d>, Without<DefaultMaterialApplied>),
>;

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
        // In diagnostic-capture mode the marker gizmos are suppressed: they are
        // debug aids, not map geometry, and they contaminate every comparison
        // shot. See `crate::shot::capture_mode`.
        let capture = crate::shot::capture_mode();
        if capture {
            info!("Diagnostic capture mode: gameplay marker gizmos disabled");
        }

        app.add_plugins(AssetsPlugin)
            .add_message::<MapLoadRequest>()
            .insert_resource(PendingMap::default())
            .insert_resource(MapAssetsRoot(self.assets_root.clone()))
            .init_resource::<MapCullConfig>()
            .insert_resource(GlobalAmbientLight {
                color: Color::srgb(1.0, 1.0, 1.0),
                // Raised from 200: with only a fill light for bounce, shadowed
                // geometry measured ~1/3 the luminance of sunlit surfaces and read
                // as black. This lifts the shadow floor without washing out the sun.
                brightness: 1_500.0,
                affects_lightmapped_meshes: true,
            })
            .insert_gizmo_config::<DefaultGizmoConfigGroup>(
                DefaultGizmoConfigGroup,
                GizmoConfig {
                    enabled: !capture,
                    line: GizmoLineConfig {
                        joints: GizmoLineJoint::Round(4),
                        ..default()
                    },
                    ..default()
                },
            )
            .add_systems(Startup, spawn_sun_light)
            .add_systems(
                Update,
                (
                    handle_map_load_request,
                    spawn_static_scene,
                    spawn_terrain_scene,
                    spawn_actor_markers,
                    spawn_map_props,
                    apply_default_materials_and_cull,
                    draw_gameplay_marker_gizmos.run_if(not_capturing),
                    debug_nearby_meshes,
                )
                    .chain(),
            );
    }
}

/// Run condition: skip a debug system while taking a diagnostic capture.
fn not_capturing() -> bool {
    !crate::shot::capture_mode()
}

fn spawn_sun_light(mut commands: Commands) {
    // Shadow cascades sized for this scene's scale. Bevy's default config covers
    // roughly a thousand units, which on a map spanning ~131k units leaves almost
    // everything outside the shadow map and the rest aliasing badly.
    let cascades = bevy::light::CascadeShadowConfigBuilder {
        num_cascades: 4,
        minimum_distance: 50.0,
        first_cascade_far_bound: 4_000.0,
        maximum_distance: 30_000.0,
        ..default()
    }
    .build();

    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: true,
            illuminance: 80_000.0,
            // Scene units are large (a building is ~500 units tall), so Bevy's
            // defaults (depth 0.02, normal 1.8) are far too small and produce
            // shadow acne — self-shadowing on surfaces facing the sun.
            shadow_depth_bias: 0.2,
            shadow_normal_bias: 40.0,
            ..default()
        },
        cascades,
        Transform::from_xyz(50_000.0, 150_000.0, 50_000.0).looking_at(Vec3::ZERO, Vec3::Y),
        GlobalTransform::IDENTITY,
    ));
    // Fill light from the opposite side. Bright enough that surfaces facing away
    // from the sun are readable rather than crushed to near-black.
    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: false,
            illuminance: 30_000.0,
            ..default()
        },
        Transform::from_xyz(-50_000.0, 80_000.0, -50_000.0).looking_at(Vec3::ZERO, Vec3::Y),
        GlobalTransform::IDENTITY,
    ));
    // Environment map for specular/reflections on PBR materials. Without an HDR
    // asset this contributes little, so it is paired with the ambient below.
    commands.spawn((
        EnvironmentMapLight {
            intensity: 2000.0,
            ..default()
        },
        GlobalTransform::IDENTITY,
    ));
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
        let props = asset_server.load(format!("{map_dir}/{map}.props.json"));

        info!(map = %map, "Loading map assets");
        *pending = PendingMap {
            gltf,
            terrain: Some(terrain),
            actors,
            props,
            spawned_static: false,
            spawned_terrain: false,
            spawned_actors: false,
            spawned_props: false,
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
        Transform::default(),
        GlobalTransform::IDENTITY,
        Visibility::default(),
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
        Transform::default(),
        GlobalTransform::IDENTITY,
        Visibility::default(),
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
    let id = pending.actors.id();
    if !asset_server.is_loaded(id) {
        return;
    }
    if let Some(state) = asset_server.get_load_state(id)
        && matches!(state, LoadState::Failed(_))
    {
        pending.spawned_actors = true;
        debug!(map = %pending.map_name, "No actor JSON for this map");
        return;
    }
    let Some(actors) = actor_assets.get(&pending.actors) else {
        return;
    };

    // UE3→glTF axis transform: (x,y,z)_ue → (x,z,y)_gltf.
    // Quaternion: swap Y/Z components to match the axis remapping.

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

        let (pos_gltf, q_gltf) = ue3_to_gltf(&actor.location, &actor.rotation);

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

/// Instantiate archetype-resolved props (flag stands, stations, turrets,
/// generators, vehicle pads, ...).
///
/// These meshes are attached through a `SkeletalMeshComponent` whose
/// `SkeletalMesh=` lives in the class default object, so they never appear in the
/// map's actor records and cannot be part of the combined static glTF. The
/// assembler resolves them from the decompiled `.uc` tree and lists the `.glb` to
/// spawn per actor in `<Map>.props.json`.
fn spawn_map_props(
    mut pending: ResMut<PendingMap>,
    prop_assets: Res<Assets<MapProps>>,
    asset_server: Res<AssetServer>,
    assets_root: Res<MapAssetsRoot>,
    mut commands: Commands,
) {
    if pending.spawned_props || pending.map_name.is_empty() {
        return;
    }
    let id = pending.props.id();
    if !asset_server.is_loaded(id) {
        return;
    }
    // Older maps have no props manifest; tolerate a failed load like terrain.
    if let Some(state) = asset_server.get_load_state(id)
        && matches!(state, LoadState::Failed(_))
    {
        pending.spawned_props = true;
        debug!(map = %pending.map_name, "No props manifest for this map");
        return;
    }
    let Some(props) = prop_assets.get(&pending.props) else {
        return;
    };

    let mut spawned = 0usize;
    for prop in &props.props {
        if prop.glb.is_empty() {
            continue;
        }
        // `#Scene0` selects the glTF's first scene as a spawnable WorldAsset.
        let scene: Handle<bevy::world_serialization::WorldAsset> =
            asset_server.load(format!("{}/{}#Scene0", assets_root.0, prop.glb));
        let (pos, rot) = ue3_to_gltf(&prop.location, &prop.rotation);
        // Scale uses the same Y/Z swap as the position.
        let scale = Vec3::new(prop.scale3d[0], prop.scale3d[2], prop.scale3d[1]);

        commands.spawn((
            WorldAssetRoot(scene),
            Transform::from_translation(pos)
                .with_rotation(rot)
                .with_scale(scale),
            GlobalTransform::IDENTITY,
            Visibility::default(),
            Name::new(format!("{} [{}]", prop.actor, prop.component)),
        ));
        spawned += 1;
    }
    pending.spawned_props = true;
    info!(
        count = spawned,
        map = %pending.map_name,
        "Spawned archetype props"
    );
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

/// For every loaded map mesh that doesn't already have a material, attach a
/// `StandardMaterial` with a per-entity pseudo-random color so the map
/// structure is visually distinguishable. Also hides meshes whose world-space
/// AABB exceeds `MapCullConfig::max_mesh_size` on any axis — these are the
/// sky-dome / occlusion-hull meshes (Perdition has a few at ~880k units).
///
/// Idempotent: tagged entities are skipped on subsequent frames.
fn apply_default_materials_and_cull(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    config: Res<MapCullConfig>,
    meshes: UnmattedMeshQuery,
) {
    let cull = config.max_mesh_size.is_finite() && config.max_mesh_size > 0.0;
    let max = config.max_mesh_size;

    let mut touched = 0usize;
    let mut hidden = 0usize;
    let mut matted = 0usize;
    for (entity, aabb, xform, existing_mat) in &meshes {
        let lo = aabb.min();
        let hi = aabb.max();
        let corners = [
            Vec3A::new(lo.x, lo.y, lo.z),
            Vec3A::new(hi.x, lo.y, lo.z),
            Vec3A::new(lo.x, hi.y, lo.z),
            Vec3A::new(hi.x, hi.y, lo.z),
            Vec3A::new(lo.x, lo.y, hi.z),
            Vec3A::new(hi.x, lo.y, hi.z),
            Vec3A::new(lo.x, hi.y, hi.z),
            Vec3A::new(hi.x, hi.y, hi.z),
        ];
        let mut wmin = Vec3A::splat(f32::INFINITY);
        let mut wmax = Vec3A::splat(f32::NEG_INFINITY);
        for c in corners {
            let w = xform.affine().transform_point3a(c);
            wmin = wmin.min(w);
            wmax = wmax.max(w);
        }
        let size = wmax - wmin;
        let is_skydome = cull && (size.x > max || size.y > max || size.z > max);

        if is_skydome {
            commands.entity(entity).insert(Visibility::Hidden);
            hidden += 1;
        } else if existing_mat.is_none() {
            // Pseudo-random per-entity color via FNV + splitmix64 on entity
            // bits, so adjacent map objects are visually distinguishable.
            let h = hash_u32(entity_to_u64(entity));
            let base = hsl_to_rgb((h % 360) as f32, 0.85, 0.55);
            let mat = materials.add(StandardMaterial {
                base_color: Color::srgb(base.0, base.1, base.2),
                perceptual_roughness: 0.7,
                metallic: 0.0,
                ..default()
            });
            commands.entity(entity).insert(MeshMaterial3d(mat));
            matted += 1;
        }
        commands.entity(entity).insert(DefaultMaterialApplied);
        touched += 1;
    }
    if touched > 0 {
        info!(
            touched,
            matted, hidden, "Processed map mesh entities (materials + cull)"
        );
    }
}

/// Encode an `Entity` as a stable `u64` for hashing. Entity has a `u32` index
/// + `u32` generation in Bevy 0.19.
fn entity_to_u64(e: Entity) -> u64 {
    let mut h: u64 = 1469598103934665603; // FNV offset basis
    for word in [e.index_u32() as u64, e.generation().to_bits() as u64] {
        for byte in word.to_le_bytes() {
            h ^= byte as u64;
            h = h.wrapping_mul(1099511628211); // FNV prime
        }
    }
    h
}

fn hash_u32(mut x: u64) -> u32 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51afd7ed558ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ceb9fe1a85ec53);
    x ^= x >> 33;
    x as u32
}

/// HSL → linear-ish RGB for `StandardMaterial::base_color`. We output in sRGB
/// and let Bevy convert, so the input is treated as sRGB by `Color::srgb`.
fn hsl_to_rgb(h_deg: f32, s: f32, l: f32) -> (f32, f32, f32) {
    let h = (h_deg.rem_euclid(360.0)) / 360.0;
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h * 6.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r1, g1, b1) = if h < 1.0 / 6.0 {
        (c, x, 0.0)
    } else if h < 2.0 / 6.0 {
        (x, c, 0.0)
    } else if h < 3.0 / 6.0 {
        (0.0, c, x)
    } else if h < 4.0 / 6.0 {
        (0.0, x, c)
    } else if h < 5.0 / 6.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    (r1 + m, g1 + m, b1 + m)
}

fn debug_nearby_meshes(
    keys: Res<ButtonInput<KeyCode>>,
    camera_q: Query<&GlobalTransform, With<Camera>>,
    meshes: Query<(
        &Aabb,
        &GlobalTransform,
        Option<&Name>,
    ), With<Mesh3d>>,
) {
    if !keys.just_pressed(KeyCode::Tab) {
        return;
    }
    let Ok(cam_xform) = camera_q.single() else {
        return;
    };
    let cam_pos = cam_xform.translation();

    const RADIUS: f32 = 2000.0;

    let mut nearby: Vec<(f32, Vec3, String)> = meshes
        .iter()
        .filter_map(|(aabb, xform, name)| {
            let center: Vec3 = xform.transform_point(aabb.center.into());
            let dist = cam_pos.distance(center);
            if dist < RADIUS {
                Some((dist, center, name.map(|n| n.to_string()).unwrap_or_default()))
            } else {
                None
            }
        })
        .collect();

    nearby.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    let mut out = format!(
        "cam=({:.1},{:.1},{:.1}) radius={RADIUS:.1} count={}\n",
        cam_pos.x, cam_pos.y, cam_pos.z, nearby.len()
    );
    for (dist, pos, name) in &nearby {
        out.push_str(&format!(
            "dist={dist:.0} pos=({:.0},{:.0},{:.0}) {name}\n",
            pos.x, pos.y, pos.z,
        ));
    }

    let _ = std::fs::write("nearby_meshes.txt", out);
    info!(count = nearby.len(), "Tab pressed — wrote nearby_meshes.txt");
}
