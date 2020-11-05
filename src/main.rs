#![recursion_limit = "256"]

#[macro_use]
extern crate diesel;

use color_eyre::eyre::Result;
use diesel::{Connection, SqliteConnection};
use dotenv::dotenv;
use std::{convert::AsRef, env, path::Path, process::Command};
use tracing::subscriber::set_global_default;
use tracing_appender::{non_blocking, rolling};
use tracing_error::ErrorLayer;
use tracing_log::LogTracer;
use tracing_subscriber::{fmt, layer::SubscriberExt, EnvFilter, Registry};

mod fs;
mod hex;
mod models;
mod schema;

pub fn establish_connection() -> SqliteConnection {
    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    SqliteConnection::establish(&database_url)
        .unwrap_or_else(|_| panic!("Error connecting to {}", database_url))
}

fn main() -> Result<()> {
    color_eyre::install()?;
    dotenv()?;
    LogTracer::init().expect("Failed to set logger");

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let formatting_layer = fmt::layer().pretty().with_writer(std::io::stderr);
    let file_appender = rolling::never("logs", "comic-fs.log");
    let (non_blocking_appender, _guard) = non_blocking(file_appender);
    let file_layer = fmt::layer()
        .with_ansi(false)
        .with_writer(non_blocking_appender);
    let subscriber = Registry::default()
        .with(env_filter)
        .with(ErrorLayer::default())
        .with(formatting_layer)
        .with(file_layer);
    set_global_default(subscriber).expect("Failed to set subscriber");
    ctrlc::set_handler(|| {
        fuse::unmount("mnt".as_ref()).expect("Fail to unmount");
    })?;

    let diesel = AsRef::<Path>::as_ref("./diesel");
    if diesel.metadata().is_ok() {
        Command::new(diesel).args(&["setup"]).status().unwrap();
    }

    let conn = establish_connection();
    fs::mount(conn, "mnt".as_ref());
    Ok(())
}
