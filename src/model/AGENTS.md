# MODEL LAYER - AGENTS.md

Business logic hub. Central `ModelController` orchestrates stores, plugins, and schedulers.

## STRUCTURE

```
model/
├── mod.rs              # ModelController: central state, Trakt client ID (line 153)
├── medias.rs           # Media lifecycle (1609 lines) - uploads, processing, encryption
├── people.rs           # Face recognition orchestration (1844 lines)
├── store.rs            # SqliteStore wrapper
├── store/sql/          # Query builders, migrations
│   ├── mod.rs          # RsQueryBuilder, SqlWhereType abstractions
│   └── library/        # Per-library schema (38 migrations)
└── plugins/            # WASM plugin model definitions
```

## KEY FILES

| File | Lines | Purpose |
|------|-------|---------|
| `people.rs` | 1844 | Face detection pipeline, clustering (Chinese Whispers), person image fallback |
| `medias.rs` | 1609 | `add_library_file`, `process_media`, encryption streams, deduplication |
| `store/sql/library/people.rs` | 1018 | Face embedding storage, weighted search, cluster promotion |

## FACE RECOGNITION FLOW

```
Media Upload → process_media_faces()
    ├── Photo: detect faces directly
    └── Video: sample frames at strategic timestamps
           ↓
    Face Detection (ONNX) → Embedding Generation
           ↓
    Match against known people OR stage for clustering
           ↓
    cluster_unassigned_faces() [Chinese Whispers]
           ↓
    Auto-promote clusters (≥10 unique media) to Person
```

## MEDIA LIFECYCLE

```
Upload → MD5 dedup check → AES encryption (if library.crypt) → Store
    ↓
process_media() spawns:
    ├── Thumbnail generation
    ├── FFprobe metadata extraction
    ├── Face detection (photos/videos)
    └── AI tag prediction
```

## DATABASE PATTERNS

- **Multi-DB**: Main `database.db` + per-library `db-{id}.db`
- **Migrations**: `include_bytes!`, `user_version` pragma tracking
- **Query Builder**: `RsQueryBuilder` with `SqlWhereType` enum (Equal, Like, In, Between, Custom)
- **Row Mapping**: Manual `row_to_*` functions, no ORM

## CONVENTIONS

- `*WithAction` wrappers for Socket.IO sync events
- `*ForAdd`, `*ForUpdate`, `*ForInsert` variants for CRUD
- Pipe-separated strings for lists (`alt` names)
- `tokio::spawn` for background processing (watch for `.unwrap()`)

## GOTCHAS

- `FaceRecognitionService` behind `Mutex` - potential contention during scans
- `person_image` has recursive resolution (local → external → face crop)
- Encryption integrated into I/O paths - increases complexity
- 512-float embeddings stored in DB - can grow rapidly
