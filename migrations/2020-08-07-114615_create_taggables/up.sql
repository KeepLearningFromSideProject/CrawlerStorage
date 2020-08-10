-- Your SQL goes here
CREATE TABLE taggables (
  id INTEGER NOT NULL PRIMARY KEY,
  tag_id INTEGER NOT NULL,
  taggable_id INTEGER NOT NULL,
  taggable_type VARCHAR NOT NULL
)
