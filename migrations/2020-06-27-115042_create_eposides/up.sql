-- Your SQL goes here
CREATE TABLE eposids (
  id INTEGER NOT NULL PRIMARY KEY,
  name VARCHAR NOT NULL,
  comic_id INTEGER NOT NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
)
