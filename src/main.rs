use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands) {
    commands.spawn(Camera3d::default());
    commands.spawn((
        Transform::from_xyz(0.0, 0.0, -5.0),
        Visibility::default(),
    ));
}
