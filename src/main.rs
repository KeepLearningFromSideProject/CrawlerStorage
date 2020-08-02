#![recursion_limit = "256"]

#[macro_use]
extern crate diesel;

use diesel::{Connection, SqliteConnection};
use dotenv::dotenv;
use std::{convert::AsRef, env, path::Path, process::Command};

mod fs;
mod models;
mod schema;

pub fn establish_connection() -> SqliteConnection {
    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    SqliteConnection::establish(&database_url)
        .unwrap_or_else(|_| panic!("Error connecting to {}", database_url))
}

fn main() {
    dotenv().ok();
    pretty_env_logger::init();
    color_backtrace::install();
    ctrlc::set_handler(|| {
        fuse::unmount("mnt".as_ref()).expect("Fail to unmount");
    })
    .expect("Fail to set ctrl c handler");

    let diesel = AsRef::<Path>::as_ref("./diesel");
    if diesel.metadata().is_ok() {
        Command::new(diesel).args(&["setup"]).status().unwrap();
    }

    let conn = establish_connection();
    fs::mount(conn, "mnt".as_ref());
}
