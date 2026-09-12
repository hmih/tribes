//! Opt-in diagnostic screenshot capture.
//!
//! Captures through an **offscreen render target** rather than the window. This
//! matters: the window's swapchain readback comes back black when the process has
//! no GUI session (which is also why `screencapture` fails from a non-interactive
//! shell), whereas rendering into an image target needs no window surface.
//!
//! Disabled unless `TASCEND_SHOT` is set, so normal runs are unaffected.
//!
//! Environment variables:
//!
//! | Variable | Meaning |
//! |---|---|
//! | `TASCEND_SHOT` | Output PNG path. Setting this enables the plugin. |
//! | `TASCEND_SHOT_DELAY` | Seconds to wait before capturing (default 15). |
//! | `TASCEND_CAM` | Optional camera placement, radians. See below. |
//! | `TASCEND_SHOT_EXIT` | `0` to keep running after the capture (default: exit). |
//!
//! `TASCEND_CAM` accepts either an explicit orientation or a look-at target:
//!
//! * `x,y,z` — position, default orientation.
//! * `x,y,z,yaw,pitch` — position and orientation, radians.
//! * `x,y,z,tx,ty,tz` — position and a world-space point to look at. Prefer this
//!   for reproducible comparison shots: it removes any yaw/pitch guesswork.
//!
//! ```bash
//! TASCEND_SHOT=/tmp/arx.png TASCEND_SHOT_DELAY=20 \
//!   TASCEND_CAM='0,20000,0,0,0,0' ./target/debug/client
//! ```

use std::path::PathBuf;

use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

use crate::flycam::Flycam;

/// True when this process was launched to take a diagnostic screenshot.
///
/// Shared by other plugins as the single source of truth for "capture mode":
/// debug-only rendering (gameplay marker gizmos, overlays) is not part of the
/// map and only pollutes the comparison.
pub fn capture_mode() -> bool {
    std::env::var_os("TASCEND_SHOT").is_some()
}

/// Frames to render into the offscreen target before the capture is requested.
const WARMUP_FRAMES: u32 = 30;
/// Frames to keep rendering after the capture, so the readback and the
/// `save_to_disk` observer complete before the app exits.
const GRACE_FRAMES: u32 = 45;
const CAPTURE_WIDTH: u32 = 1600;
const CAPTURE_HEIGHT: u32 = 900;

pub struct DiagnosticShotPlugin;

#[derive(Resource)]
struct ShotConfig {
    path: PathBuf,
    delay: f32,
    camera: Option<(Vec3, f32, f32)>,
    exit_after: bool,
    target: Handle<Image>,
}

#[derive(Resource, Default)]
struct ShotState {
    elapsed: f32,
    camera_spawned: bool,
    warmup: u32,
    requested: bool,
    frames_since_request: u32,
}

impl Plugin for DiagnosticShotPlugin {
    fn build(&self, app: &mut App) {
        // Absent env var => plugin does nothing at all.
        let Ok(path) = std::env::var("TASCEND_SHOT") else {
            return;
        };

        let delay = std::env::var("TASCEND_SHOT_DELAY")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(15.0);

        let camera = std::env::var("TASCEND_CAM").ok().and_then(|v| {
            let f: Vec<f32> = v
                .split(',')
                .filter_map(|p| p.trim().parse().ok())
                .collect();
            let at = |i: usize| Vec3::new(f[i], f[i + 1], f[i + 2]);
            match f.len() {
                3 => Some((at(0), 0.0, -1.2)),
                5 => Some((at(0), f[3], f[4])),
                6 => {
                    // Position + look-at. The flycam's forward vector is
                    // `Ry(yaw) * Rx(pitch) * -Z`, i.e. `(-sin yaw, sin pitch,
                    // -cos yaw)` for a unit direction, which inverts to the
                    // expressions below. Keeping `Flycam::yaw/pitch` in sync
                    // matters because the look system rebuilds the rotation from
                    // them on any mouse input.
                    let pos = at(0);
                    let dir = (at(3) - pos).normalize_or_zero();
                    let yaw = (-dir.x).atan2(-dir.z);
                    let pitch = dir.y.clamp(-1.0, 1.0).asin();
                    Some((pos, yaw, pitch))
                }
                _ => {
                    warn!(
                        "TASCEND_CAM must be 'x,y,z', 'x,y,z,yaw,pitch' or \
                         'x,y,z,tx,ty,tz'; ignoring"
                    );
                    None
                }
            }
        });

        let exit_after = std::env::var("TASCEND_SHOT_EXIT")
            .map(|v| v != "0")
            .unwrap_or(true);

        let target = {
            let mut images = app.world_mut().resource_mut::<Assets<Image>>();
            let mut image = Image::new_target_texture(
                CAPTURE_WIDTH,
                CAPTURE_HEIGHT,
                TextureFormat::Rgba8UnormSrgb,
                None,
            );
            // The screenshot path copies out of this texture.
            image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
            images.add(image)
        };

        info!(
            path = %path,
            delay,
            exit_after,
            size = format!("{CAPTURE_WIDTH}x{CAPTURE_HEIGHT}"),
            "Diagnostic screenshot enabled (offscreen render target)"
        );

        app.insert_resource(ShotConfig {
            path: PathBuf::from(path),
            delay,
            camera,
            exit_after,
            target,
        })
        .init_resource::<ShotState>()
        .add_systems(Update, drive_shot);
    }
}

fn drive_shot(
    time: Res<Time>,
    config: Res<ShotConfig>,
    mut state: ResMut<ShotState>,
    mut flycam_camera: Query<(&Transform, &mut Flycam, &Projection)>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    if state.requested {
        state.frames_since_request += 1;
        if config.exit_after && state.frames_since_request > GRACE_FRAMES {
            exit.write(AppExit::Success);
        }
        return;
    }

    if state.camera_spawned {
        state.warmup += 1;
        if state.warmup < WARMUP_FRAMES {
            return;
        }
        info!(path = %config.path.display(), "Capturing screenshot");
        commands
            .spawn(Screenshot::image(config.target.clone()))
            .observe(save_to_disk(config.path.clone()));
        state.requested = true;
        return;
    }

    state.elapsed += time.delta_secs();
    if state.elapsed < config.delay {
        return;
    }

    // Mirror the flycam's view into an offscreen camera. The override, when set,
    // also updates the flycam so its per-frame look system stays consistent.
    let Ok((transform, mut flycam, projection)) = flycam_camera.single_mut() else {
        return;
    };

    let (position, rotation) = match config.camera {
        Some((pos, yaw, pitch)) => {
            flycam.yaw = yaw;
            flycam.pitch = pitch;
            (
                pos,
                Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch),
            )
        }
        None => (transform.translation, transform.rotation),
    };

    commands.spawn((
        Camera3d::default(),
        Camera::default(),
        RenderTarget::Image(config.target.clone().into()),
        Transform::from_translation(position).with_rotation(rotation),
        GlobalTransform::from(Transform::from_translation(position).with_rotation(rotation)),
        projection.clone(),
        Name::new("diagnostic-shot-camera"),
    ));
    state.camera_spawned = true;
    info!("Offscreen capture camera spawned");
}
