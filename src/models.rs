use crate::schema::{self, comics, eposides, files, taggables, tags};
use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::Deserialize;

#[derive(Queryable, Debug)]
pub struct Comic {
    pub id: i32,
    pub name: String,
    pub created_at: NaiveDateTime,
}

impl Comic {
    pub fn find(id: i32, conn: &SqliteConnection) -> Option<Self> {
        use schema::comics::dsl;

        dsl::comics.find(id).first::<Comic>(conn).ok()
    }

    pub fn find_by_name(name: &str, conn: &SqliteConnection) -> Option<Self> {
        use schema::comics::dsl;

        dsl::comics
            .filter(dsl::name.eq(name))
            .first::<Comic>(conn)
            .ok()
    }
}

#[derive(Queryable)]
pub struct Eposide {
    pub id: i32,
    pub name: String,
    pub comic_id: i32,
    pub created_at: NaiveDateTime,
}

impl Eposide {
    pub fn find(id: i32, conn: &SqliteConnection) -> Option<Self> {
        use schema::eposides::dsl;

        dsl::eposides.find(id).first::<Self>(conn).ok()
    }

    pub fn find_by_comic_and_name(
        comic_id: i32,
        name: &str,
        conn: &SqliteConnection,
    ) -> Option<Self> {
        use schema::eposides::dsl;

        dsl::eposides
            .filter(dsl::comic_id.eq(comic_id))
            .filter(dsl::name.eq(name))
            .first::<Eposide>(conn)
            .ok()
    }
}

#[derive(Queryable, Identifiable)]
pub struct File {
    pub id: i32,
    pub name: String,
    pub content_hash: String,
    pub eposid_id: i32,
    pub access_count: i32,
    pub created_at: NaiveDateTime,
}

impl File {
    pub fn find(id: i32, conn: &SqliteConnection) -> Option<Self> {
        use schema::files::dsl;

        dsl::files.find(id).first::<File>(conn).ok()
    }

    pub fn find_by_eposide_and_name(
        eposide_id: i32,
        name: &str,
        conn: &SqliteConnection,
    ) -> Option<Self> {
        use schema::files::dsl;

        dsl::files
            .filter(dsl::eposid_id.eq(eposide_id))
            .filter(dsl::name.eq(name))
            .first::<File>(conn)
            .ok()
    }

    pub fn update_content_hash(&self, content_hash: &str, conn: &SqliteConnection) {
        use schema::files::dsl;

        diesel::update(self)
            .set(dsl::content_hash.eq(content_hash))
            .execute(conn)
            .unwrap();
    }
}

#[derive(Queryable)]
pub struct Tag {
    pub id: i32,
    pub name: String,
    pub created_at: NaiveDateTime,
}

impl Tag {
    pub fn list(conn: &SqliteConnection) -> Option<Vec<Self>> {
        tags::table.load::<Self>(conn).ok()
    }

    pub fn find(id: i32, conn: &SqliteConnection) -> Option<Self> {
        tags::table
            .filter(tags::dsl::id.eq(id))
            .first::<Self>(conn)
            .ok()
    }

    pub fn find_by_name(name: &str, conn: &SqliteConnection) -> Option<Self> {
        use tags::dsl;

        dsl::tags
            .filter(dsl::name.eq(name))
            .first::<Self>(conn)
            .ok()
    }
}

#[derive(Queryable)]
pub struct Taggable {
    pub id: i32,
    pub tag_id: i32,
    pub taggable_id: i32,
    pub taggable_type: String,
}

#[derive(strum_macros::EnumString)]
enum TaggableKind {
    Comic,
    Eposide,
    File,
}

pub enum Taggables {
    Comic(Comic),
    Eposide(Eposide),
    File(File),
}

impl Taggables {
    pub fn taggables(id: i32, conn: &SqliteConnection) -> Vec<Self> {
        use taggables::dsl;

        dsl::taggables
            .filter(dsl::tag_id.eq(id))
            .load::<Taggable>(conn)
            .map(|taggables| {
                taggables
                    .iter()
                    .filter_map(|taggable| {
                        let kind = taggable.taggable_type.parse::<TaggableKind>().ok()?;
                        match kind {
                            TaggableKind::Comic => {
                                use comics::dsl;

                                dsl::comics
                                    .filter(dsl::id.eq(taggable.taggable_id))
                                    .first::<Comic>(conn)
                                    .ok()
                                    .map(Taggables::Comic)
                            }
                            TaggableKind::Eposide => {
                                use eposides::dsl;

                                dsl::eposides
                                    .filter(dsl::id.eq(taggable.taggable_id))
                                    .first::<Eposide>(conn)
                                    .ok()
                                    .map(Taggables::Eposide)
                            }
                            TaggableKind::File => {
                                use files::dsl;

                                dsl::files
                                    .filter(dsl::id.eq(taggable.taggable_id))
                                    .first::<File>(conn)
                                    .ok()
                                    .map(Taggables::File)
                            }
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|_| Vec::new())
    }
}

#[derive(Deserialize, Insertable)]
#[table_name = "comics"]
pub struct NewComic<'a> {
    pub name: &'a str,
}

#[derive(Deserialize, Insertable)]
#[table_name = "eposides"]
pub struct NewEposide<'a> {
    pub name: &'a str,
    pub comic_id: i32,
}

#[derive(Deserialize, Insertable)]
#[table_name = "files"]
pub struct NewFile<'a> {
    pub name: &'a str,
    pub content_hash: &'a str,
    pub eposid_id: i32,
}

impl NewFile<'_> {
    pub fn insert(self, conn: &SqliteConnection) -> Result<File, diesel::result::Error> {
        conn.transaction::<_, diesel::result::Error, _>(|| {
            use files::dsl;

            diesel::insert_into(files::table)
                .values(&self)
                .execute(conn)?;
            Ok(dsl::files.order(dsl::id.desc()).first::<File>(conn)?)
        })
    }
}

#[derive(Deserialize, Insertable)]
#[table_name = "tags"]
pub struct NewTag<'a> {
    pub name: &'a str,
}

impl NewTag<'_> {
    pub fn insert(self, conn: &SqliteConnection) -> Result<Tag, diesel::result::Error> {
        conn.transaction::<_, diesel::result::Error, _>(|| {
            use tags::dsl;

            diesel::insert_into(tags::table)
                .values(&self)
                .execute(conn)?;
            Ok(dsl::tags.order(dsl::id.desc()).first::<Tag>(conn)?)
        })
    }
}
