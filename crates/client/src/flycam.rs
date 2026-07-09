//! First-person flycam for inspecting loaded maps.
//!
//! Movement: WASD + Space/Shift (up/down). Look: mouse while right-button
//! held. Sprint: hold Ctrl. No collision — purely for visual inspection.

use std::f32::consts::FRAC_PI_2;

use bevy::input::mouse::MouseMotion;
use bevy::prelude::*;
use bevy::window::CursorGrabMode;

/// Marker component for the flycam entity. Carries yaw/pitch so we can clamp
/// pitch independently of the `Transform` quaternion.
#[derive(Component)]
pub struct Flycam {
    pub yaw: f32,
    pub pitch: f32,
}

/// Configuration for the flycam plugin.
#[derive(Resource)]
pub struct FlycamConfig {
    /// Spawn position (in glTF/world space).
    pub start_position: Vec3,
    /// Initial yaw (radians, around +Y world up).
    pub start_yaw: f32,
    /// Initial pitch (radians, around local X).
    pub start_pitch: f32,
    /// Base movement speed (units/second).
    pub move_speed: f32,
    /// Sprint multiplier when Ctrl is held.
    pub sprint_multiplier: f32,
    /// Mouse sensitivity (radians per pixel of motion).
    pub sensitivity: f32,
    /// Cursor grabbed while looking?
    pub grab_cursor: bool,
}

impl Default for FlycamConfig {
    fn default() -> Self {
        Self {
            // Spawn above the map looking down. The map (excluding ~880k-unit
            // skydomes, which are culled) spans about ±33k units; start 25k up
            // at a steep pitch so the whole arena is visible on first frame.
            start_position: Vec3::new(0.0, 25_000.0, 0.0),
            start_yaw: 0.0,
            start_pitch: -1.2,
            move_speed: 3_000.0,
            sprint_multiplier: 8.0,
            sensitivity: 0.0025,
            grab_cursor: true,
        }
    }
}

pub struct FlycamPlugin;

impl Plugin for FlycamPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FlycamConfig>()
            .add_systems(Startup, spawn_flycam)
            .add_systems(Update, (flycam_look, flycam_move).chain());
    }
}

fn spawn_flycam(mut commands: Commands, config: Res<FlycamConfig>) {
    let yaw_q = Quat::from_rotation_y(config.start_yaw);
    let pitch_q = Quat::from_rotation_x(config.start_pitch);
    let rotation = yaw_q * pitch_q;

    commands.spawn((
        Flycam {
            yaw: config.start_yaw,
            pitch: config.start_pitch,
        },
        Transform::from_translation(config.start_position).with_rotation(rotation),
        GlobalTransform::from(
            Transform::from_translation(config.start_position).with_rotation(rotation),
        ),
        Visibility::default(),
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            fov: 60.0f32.to_radians(),
            near: 1.0,
            far: 500_000.0,
            ..default()
        }),
    ));
}

fn flycam_look(
    mut motion_events: MessageReader<MouseMotion>,
    mouse_button: Res<ButtonInput<MouseButton>>,
    mut query: Query<(&mut Flycam, &mut Transform)>,
    config: Res<FlycamConfig>,
    mut cursor: Query<&mut bevy::window::CursorOptions>,
) {
    if !mouse_button.pressed(MouseButton::Right) {
        return;
    }
    let total: Vec2 = motion_events.read().map(|e| e.delta).sum();
    if total == Vec2::ZERO {
        return;
    }
    if let Ok(mut opts) = cursor.single_mut()
        && config.grab_cursor
    {
        opts.grab_mode = CursorGrabMode::Locked;
        opts.visible = false;
    }
    let Ok((mut fly, mut tf)) = query.single_mut() else {
        return;
    };
    fly.yaw -= total.x * config.sensitivity;
    fly.pitch -= total.y * config.sensitivity;
    let limit = FRAC_PI_2 - 0.01;
    fly.pitch = fly.pitch.clamp(-limit, limit);

    let yaw_q = Quat::from_rotation_y(fly.yaw);
    let pitch_q = Quat::from_rotation_x(fly.pitch);
    tf.rotation = yaw_q * pitch_q;
}

fn flycam_move(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    config: Res<FlycamConfig>,
    mut query: Query<(&Flycam, &mut Transform)>,
) {
    let Ok((fly, mut tf)) = query.single_mut() else {
        return;
    };
    let yaw_q = Quat::from_rotation_y(fly.yaw);
    let forward = yaw_q * Vec3::NEG_Z;
    let right = yaw_q * Vec3::X;

    let mut dir = Vec3::ZERO;
    if keys.pressed(KeyCode::KeyW) {
        dir += forward;
    }
    if keys.pressed(KeyCode::KeyS) {
        dir -= forward;
    }
    if keys.pressed(KeyCode::KeyD) {
        dir += right;
    }
    if keys.pressed(KeyCode::KeyA) {
        dir -= right;
    }
    if keys.pressed(KeyCode::Space) {
        dir += Vec3::Y;
    }
    if keys.pressed(KeyCode::ShiftLeft) {
        dir -= Vec3::Y;
    }
    if dir == Vec3::ZERO {
        return;
    }
    let speed = config.move_speed
        * if keys.pressed(KeyCode::ControlLeft) {
            config.sprint_multiplier
        } else {
            1.0
        };
    let dir = dir.normalize_or_zero() * speed * time.delta_secs();
    tf.translation += dir;
}
