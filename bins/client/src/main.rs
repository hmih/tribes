use bevy::asset::AssetPlugin;
use bevy::prelude::*;
use tribes_client::{FlycamPlugin, MapLoadRequest, MapViewerPlugin};

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(AssetPlugin {
            // Run from src/port/ so assets resolve to ../../decompile/assets/
            file_path: "bins/client".into(),
            ..default()
        }))
        .add_plugins(FlycamPlugin)
        .add_plugins(MapViewerPlugin::default())
        .add_systems(Startup, load_perdition)
        .run();
}

fn load_perdition(mut writer: bevy::ecs::message::MessageWriter<MapLoadRequest>) {
    writer.write(MapLoadRequest("Perdition".into()));
}
