# Model layer

`ModelController` in `mod.rs` coordinates business logic, stores, plugins, and
background work. Root guidance applies; paths below are relative to this directory.

## Entry points

- `medias.rs`: uploads, deduplication, processing, and encrypted media I/O.
- `people.rs`: face processing, matching, clustering, and person image handling.
- `entity_images.rs`: shared entity image lookup and download helpers.
- `store.rs`: main `database.db` and per-library `db-{id}.db` connections.
- `store/sql/mod.rs`: shared query builders and main database migrations.
- `store/sql/library/mod.rs`: library migrations; sibling entity modules contain
  SQL and manual `row_to_*` mappings, including face storage in `people.rs`.

## Change constraints

- Keep library/user permission checks in model operations; callers include more
  than HTTP handlers.
- For schema changes, add the next numbered SQL migration in the appropriate
  database directory and register it in that directory's `mod.rs`, following
  `include_bytes!` and `user_version` handling. Preserve upgrades from existing DBs.
- Keep selected column order and `row_to_*` mappings aligned. Reuse query builders
  and serialization helpers, including pipe-separated list fields where used.
- Respect timestamp triggers and sync ordering when changing persistence.
- Follow existing `*ForAdd`, `*ForUpdate`, `*ForInsert`, and `*WithAction` types;
  keep mutation events consistent with persisted data and the root event-doc rule.
- Media lifecycle changes must account for deduplication, encryption, and
  background processing. Face changes may span `people.rs`, the library store,
  `../tools/recognition.rs`, and `../tools/scheduler/face_recognition.rs`; check
  those boundaries when changing matching or clustering behavior.
