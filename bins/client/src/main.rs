#![allow(unused_imports)]

use bevy::app::App;
use bevy::DefaultPlugins;
use log::info;

fn main() {
    info!("Tribes client starting");
    let mut app = App::new();
    app.add_plugins(DefaultPlugins);
    app.run();
}