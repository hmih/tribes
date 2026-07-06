#![allow(unused_imports)]

use log::info;
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).init();
    info!("Tribes realm server starting");

    let _pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect("sqlite:tribes_realm.db?mode=rwc")
        .await?;

    info!("Realm server running (gRPC not yet configured)");
    tokio::signal::ctrl_c().await?;
    info!("Shutting down");
    Ok(())
}