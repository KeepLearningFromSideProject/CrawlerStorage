#![recursion_limit = "256"]

#[macro_use]
extern crate diesel;

use diesel::{Connection, SqliteConnection};
use dotenv::dotenv;
use std::env;

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

    let conn = establish_connection();
    fs::mount(conn, "mnt".as_ref());
}
