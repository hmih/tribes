use bevy::asset::AssetPlugin;
use bevy::prelude::*;
use bevy::transform::components::{GlobalTransform, Transform};
use tribes_client::{FlycamPlugin, MapLoadRequest, MapViewerPlugin};

#[allow(unused_imports)]
#[cfg(debug_assertions)]
use bevy_dylib;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(AssetPlugin {
            // Asset root: src/decompile/assets/ (relative to bins/client manifest dir)
            file_path: "../../../decompile/assets".into(),
            ..default()
        }))
        // Safety net: bevy_transform 0.19 does not call register_type in its
        // plugin, and reflect_auto_register_static failed to pick it up. Even
        // with reflect_auto_register (inventory-based) enabled, register these
        // explicitly so scene spawning never panics on "unregistered type".
        .register_type::<Transform>()
        .register_type::<GlobalTransform>()
        .add_plugins(FlycamPlugin)
        .add_plugins(MapViewerPlugin::default())
        .add_systems(Startup, load_perdition)
        .run();
}

fn load_perdition(mut writer: bevy::ecs::message::MessageWriter<MapLoadRequest>) {
    writer.write(MapLoadRequest("Perdition".into()));
}
