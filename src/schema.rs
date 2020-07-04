table! {
    comics (id) {
        id -> Integer,
        name -> Text,
        created_at -> Timestamp,
    }
}

table! {
    eposides (id) {
        id -> Integer,
        name -> Text,
        comic_id -> Integer,
        created_at -> Timestamp,
    }
}

table! {
    files (id) {
        id -> Integer,
        name -> Text,
        content_hash -> Text,
        eposid_id -> Integer,
        access_count -> Integer,
        created_at -> Timestamp,
    }
}

allow_tables_to_appear_in_same_query!(
    comics,
    eposides,
    files,
);
