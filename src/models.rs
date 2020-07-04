use crate::schema::files;
use chrono::NaiveDateTime;
use diesel::{Insertable, Queryable};
use serde::Deserialize;

#[derive(Queryable, Debug)]
pub struct Comic {
    pub id: i32,
    pub name: String,
    pub created_at: NaiveDateTime,
}

#[derive(Queryable)]
pub struct Eposide {
    pub id: i32,
    pub name: String,
    pub comic_id: i32,
    pub created_at: NaiveDateTime,
}

#[derive(Queryable)]
pub struct File {
    pub id: i32,
    pub name: String,
    pub content_hash: String,
    pub eposid_id: i32,
    pub access_count: i32,
    pub created_at: NaiveDateTime,
}

#[derive(Deserialize, Insertable)]
#[table_name = "files"]
pub struct NewFile<'a> {
    pub name: &'a str,
    pub content_hash: &'a str,
    pub eposid_id: i32,
}
