#![allow(unused_imports)]

use bevy::app::App;
use bevy::MinimalPlugins;
use log::info;

fn main() {
    info!("Tribes game server starting");
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.run();
}