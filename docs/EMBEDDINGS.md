# Media embeddings and semantic search

The server can compute one vector (an *embedding*) per photo and video, and use it
to find medias from a text query ("kids at the beach") or medias that look like a
given one. Everything runs locally on CPU.

This is separate from tag prediction (`PluginType::ImageClassification`,
`src/tools/prediction.rs`), which is a fixed-label classifier writing `ai/<tag>`
tags. The two do not share code or data.

## Where the code is

| Part | Location |
| --- | --- |
| `Embedder` trait, int8 conversion | `src/tools/embedding/mod.rs` |
| EmbeddingGemma 2 implementation, model download | `src/tools/embedding/gemma.rs` |
| In-memory search index | `src/tools/embedding/index.rs` |
| Table and queries | `src/model/store/sql/library/embeddings.rs`, migration `062 - MEDIA EMBEDDINGS.sql` |
| Embedding a media, search, access rules | `src/model/embeddings.rs` |
| Background task | `src/tools/scheduler/embedding.rs` |
| Routes | `src/routes/medias.rs` |

`src/tools/embedding/` is self-contained on purpose: it imports nothing from
`crate::model`, the routes or the store, and has its own error type. There is no
`lib.rs` in this repository, so this is what lets another binary (a worker running
on a faster machine) re-declare the module and reuse it unchanged.

## How it works

1. A library opts in with the `embeddings` setting (see [Turning it on](#turning-it-on)).
2. Every 10 minutes the `Embedding` scheduler task asks each opted-in library for
   its *pending* medias: photos and videos with no vector for the active embedder.
   New medias are picked up this way, nothing else has to trigger it.
3. Each media is embedded and the vector stored:
   - **Photo**: the server's decoded image (`media_image`, the same image used for
     thumbnails), not the full-resolution original. The model resizes its input to
     roughly 624×480 anyway.
   - **Video**: 8 frames spread evenly over the duration are each embedded as an
     image; the float vectors are averaged, re-normalised, and stored as one vector.
     If no frame can be extracted the video thumbnail is used instead.
4. A search embeds the query text on the server, then scans the library's vectors.

The model is downloaded on first use (about 316 MB) into
`<config dir>/models/embeddinggemma-2-q4/`. It lives in the configuration directory
rather than the working directory so that a Docker volume keeps it.

The download is pinned to one commit of the model repository, and each file is
checked against its SHA-256 before it is kept. The embedder id promises that every
stored vector comes from the same weights, so moving to another revision of the
files needs a new id.

### Cost

Measured on 6 Haswell cores (AVX2), 4 threads: about 3 s per image, of which 2.7 s
is the vision encoder, so a video costs about 25 s. A text query takes about 45 ms.
Both sessions loaded use about 0.9 GB.

The text backbone stays loaded once used, so searches stay fast. The vision encoder
is loaded when a pass starts embedding and released when the pass ends. Embedding
uses up to 4 threads and only one pass runs at a time.

## The embedder abstraction

```rust
pub trait Embedder: Send + Sync {
    fn spec(&self) -> &EmbedderSpec;          // id, dimension, search_dimension, quantization_scale
    fn embed_image(&self, bytes: &[u8]) -> EmbeddingResult<Vec<f32>>;
    fn embed_text(&self, query: &str) -> EmbeddingResult<Vec<f32>>;
    fn release_image_resources(&self) {}
}
```

Both methods return L2-normalised float vectors in the same space. Storage, the
index and search only ever see vectors and the `EmbedderSpec`; nothing outside
`gemma.rs` knows about tokens, patches or ONNX graphs.

### The embedder id rule

Every stored vector carries the id of the embedder that produced it, and search
only compares vectors that have the active id. Vectors from different models,
weight precisions or input budgets are not comparable, so the id encodes all three:

```
embeddinggemma-2/q4/t140
└ model          │  └ 140 vision tokens per image
                 └ q4 weights
```

**Any change that alters the stored numbers needs a new id**, including a change of
the int8 scale below. After an id change the old rows are simply ignored and every
media becomes pending again; old rows can be deleted with
`DELETE FROM media_embeddings WHERE embedder = '<old id>'`.

### Adding another embedder

1. Implement `Embedder` in a new file under `src/tools/embedding/`, with a `SPEC`
   constant. Set `search_dimension` equal to `dimension` unless the model is trained
   so that a prefix of the vector is usable on its own.
2. Derive its `quantization_scale` (see below) and hard-code it next to the id.
3. Point `active_spec`, `active_files_present` and `load_active` in
   `src/tools/embedding/mod.rs` at it.

Nothing else changes: the table, the pending query, the index and the API are keyed
by the id and sized by the spec.

## Storage

Per-library database, migration 062:

```sql
CREATE TABLE media_embeddings (
    media_ref TEXT NOT NULL,
    embedder TEXT NOT NULL,
    embedding BLOB NOT NULL,   -- int8, one byte per value
    PRIMARY KEY (media_ref, embedder)
) WITHOUT ROWID;

CREATE TABLE media_embedding_errors (
    media_ref TEXT NOT NULL,
    embedder TEXT NOT NULL,
    error TEXT,
    attempts INTEGER NOT NULL DEFAULT 1,
    modified INTEGER NOT NULL,  -- ms, time of the last attempt
    PRIMARY KEY (media_ref, embedder)
) WITHOUT ROWID;
```

A vector is 768 signed bytes (two's complement), about 82 MB of database per
100 000 medias. A trigger on `medias` deletes both kinds of rows with the media.

### int8 conversion

```
stored = clamp(round(v * S), -127, 127)
```

with one fixed scale `S` per embedder. For `embeddinggemma-2/q4/t140`,
**`S = 408.6`**.

It was derived as `S = 127 / p`, where `p = 0.3108` is the 99.9th percentile of
the absolute component values over a sample of real embeddings: 62 photos (60 COCO
val2017 images and 2 of this repository's test images, at 140 tokens) and 40 text
queries, 78 336 values in total. Images alone give 0.3038 and texts alone 0.3154.
The measurement is the ignored test `embeddinggemma_calibration` in
`src/tools/embedding/gemma.rs`:

```bash
REDSEAT_EMBEDDING_CALIBRATION_DIR=/path/to/images \
  cargo test --bin redseat-rust embeddinggemma_calibration -- --ignored --nocapture
```

The percentile is high because this model puts a few large values at the same
positions in every vector (for example about 0.32 at index 115, while the median
absolute value is 0.017). Those shared values are what makes unrelated content
still score around 0.5. A larger scale that clips them does not just add noise, it
shifts every score: at `S = 1090` the reference pair went from 0.76 to 0.64. So the
scale is part of what the id stands for: do not change it without changing the id.

On the same sample, the stored design (convert all 768 values to int8, search the
first 512) and the alternative (truncate to 512, re-normalise, then convert)
differ by at most 0.007 in cosine (0.002 on average) and agree on the best image
for 39 of 40 queries. Against float vectors the int8 score is off by at most 0.025.

### Failures

A media that cannot be embedded gets a row in `media_embedding_errors`. It is
offered again by later passes, at most once per pass, and is given up on after
3 attempts. A success deletes the row. `failed` in the status below counts the
medias given up on; `POST .../embeddings/process?retryFailed=true` forgets the
failures of the library.

Two guards keep a problem that is not about the medias from being charged to them:

- A pass loads the whole model, vision encoder included, before it touches a media.
  A model that cannot load stops the pass with no failure recorded.
- After 5 failures in a row a pass leaves the library until the next one. A broken
  model or unreachable storage then costs those 5 medias an attempt, not the whole
  library.

### Pending medias

"What is left to embed" is one store method,
`SqliteLibraryStore::get_medias_pending_embedding(embedder, max_attempts, retry_failed_before, limit)`:
photos and videos with no row for the embedder, minus the ones given up on. The
background task takes its work from it, and so should anything else that embeds
medias.

## Search

Search uses the first `search_dimension` (512) values of the stored int8 vectors.
EmbeddingGemma is trained so that leading dimensions work on their own; an
embedder that is not must declare `search_dimension == dimension`.

The score is the cosine similarity of the two 512-value prefixes, computed as an
integer dot product divided by the two prefix norms. The query goes through the
same int8 conversion. Text queries are embedded with the prefix
`task: search result | query: `; images get no prefix.

Each library has an in-memory index: the 512-byte prefixes in one contiguous
buffer with their norms pre-computed, about 51 MB per 100 000 medias. It is loaded
on the first search of the library, updated as vectors are added and medias
deleted, and dropped with the library. A search is a plain linear scan, about
21 ms per 100 000 medias on one thread; there is no approximate index.

## Turning it on

Embeddings are off by default. Set `embeddings` in the library settings:

```
PATCH /libraries/{libraryId}
{ "settings": { ...existing settings, "embeddings": true } }
```

Turning it off stops the background work once the media in progress is done.
Existing vectors are kept and stay searchable.

### Pausing

A library can be paused without turning embeddings off:

```
POST /libraries/{libraryId}/medias/embeddings/pause
POST /libraries/{libraryId}/medias/embeddings/resume
```

- While paused, nothing is embedded in the library: passes skip it and a manual
  start is refused. Search keeps working on the vectors already stored.
- A pass that is running leaves the library once the media in progress is done, and
  reports `paused`.
- Resuming queues a pass right away. It carries on with what is left, since pending
  medias are always read from the database.
- The pause is the library setting `embeddingsPaused`, so it survives a restart.
  The two endpoints change only that setting and keep the others.

Libraries with client-side encryption (`crypt`) are skipped: the server cannot read
their files. Password-encrypted libraries work, the server decrypts them as it does
for thumbnails.

## API

All routes are under `/libraries/{libraryId}/medias`.

### `GET /embeddings/search?text=...&limit=50&offset=0`

Medias closest to a text query, best first. `limit` defaults to 50 (maximum 500).

```json
{
  "status": "ok",
  "embedder": "embeddinggemma-2/q4/t140",
  "results": [
    { "media": { "id": "...", "name": "beach.jpg", "type": "photo" }, "score": 0.71 }
  ]
}
```

`media` has the same shape as an item of `GET /libraries/{libraryId}/medias`.
Results go through the same access rules as that listing (library role, per-user
limits, upload keys).

`status` tells an empty list apart from "no match":

| `status` | Meaning |
| --- | --- |
| `ok` | `results` is the ranked list |
| `noEmbeddings` | Nothing is embedded yet in this library for the active embedder |
| `modelDownloading` | The model is being downloaded; retry later |
| `mediaNotEmbedded` | (`similar` only) the reference media has no vector yet |

When `status` is not `ok`, `results` is empty and `message` explains why.

### `GET /{mediaId}/similar?limit=50&offset=0`

Medias closest to the given one, which is left out of the results. Same response.
Uses stored vectors only, so it works even when the model is not loaded.

The reference media goes through the same listing scope as the results: one the
user cannot list is answered as not found.

### `GET /embeddings/status`

```json
{
  "embedder": "embeddinggemma-2/q4/t140",
  "dimension": 768,
  "searchDimension": 512,
  "enabled": true,
  "paused": false,
  "model": "loaded",
  "running": false,
  "total": 12000,
  "embedded": 11950,
  "pending": 48,
  "failed": 2
}
```

`model` is `loaded`, `available` (on disk, loaded on first use), `downloading` or
`missing`. `total` counts the photos and videos of the library. `running` is true
while a pass is running on the server, for any library. `paused` is the library's
pause, see above.

### `POST /embeddings/process?retryFailed=false`

Library admin only. Queues a pass on this library right away instead of waiting for
the next one. The library must have `embeddings` on and not be paused.

The response `status` is `started`, or `alreadyRunning` when a pass is in progress.
Passes do not overlap: the library is then handled by the running pass, or by the
next recurring one (at most 10 minutes after it ends) if that pass is already past
it. With `retryFailed=true` the failures are forgotten in both cases.

### `POST /embeddings/pause` and `POST /embeddings/resume`

Library admin only. See "Pausing" above. Both answer with the status object of
`GET /embeddings/status`, as it is after the change.

### Progress events

While a pass runs on a library, `embedding_progress` SSE events carry its progress
to anyone with read access to that library:

```json
{
  "embeddingProgress": {
    "library": "<library id>",
    "state": "running",
    "embedder": "embeddinggemma-2/q4/t140",
    "processed": 12,
    "embedded": 11,
    "total": 5400
  }
}
```

`state` is `started` (sent once, before the model is loaded), `running` (after each
media), then exactly one of `completed`, `paused` (the library was paused),
`interrupted` (left after repeated failures) or `failed` (the pass could not run,
for example when the model cannot be downloaded or loaded; the reason is in the
server log). A pass that announced its start always sends one of these last four.
See `docs/SSE.md`.

## Known limits

- **Scores are compressed.** With this model a good match scores around 0.69 to
  0.75 and unrelated content around 0.60 to 0.64. Results are a ranked list; there
  is no reliable score below which a result is "not a match", so the API applies
  no cut-off.
- **No content filtering.** A neutral query can surface adult content if the
  library contains it. Nothing flags or filters it yet.
- **One vector per video**, from 8 frames. A short scene in a long video is easily
  missed.
- **Vectors are not refreshed** when a media's image is replaced. Delete its row
  in `media_embeddings` to have it embedded again.
- **Photos are embedded from the server's thumbnail-sized image**, so very small
  details of a large photo are lost.
- **CPU only and slow**: about 3 s per image. A large library takes days to
  backfill.
- **No Intel macOS build.** The model needs ONNX Runtime 1.23 or later, and `ort`
  ships no prebuilt runtime for `x86_64-apple-darwin` at those versions.
