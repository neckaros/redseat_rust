# Media embeddings and semantic search

The server can compute one vector (an *embedding*) per photo and video, and use it
to find medias from a text query ("kids at the beach") or medias that look like a
given one. Everything runs locally: on the CPU, or for images on the GPU in a build
that has it (see [GPU](#gpu)).

This is separate from tag prediction (`PluginType::ImageClassification`,
`src/tools/prediction.rs`), which is a fixed-label classifier writing `ai/<tag>`
tags. The two do not share code or data.

## Where the code is

| Part | Location |
| --- | --- |
| `Embedder` trait, int8 conversion | `src/tools/embedding/mod.rs` |
| EmbeddingGemma 2 implementation, model download, CPU or GPU | `src/tools/embedding/gemma.rs` |
| In-memory search index | `src/tools/embedding/index.rs` |
| Table and queries | `src/model/store/sql/library/embeddings.rs`, migration `062 - MEDIA EMBEDDINGS.sql` |
| Embedding a media, search, access rules | `src/model/embeddings.rs` |
| Text search in the media listing | `search_medias_hybrid` in `src/model/embeddings.rs`, called by `get_medias` in `src/model/medias.rs` |
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
   - **Video**: frames spread evenly over the duration are each embedded as an
     image; the float vectors are averaged, re-normalised, and stored as one vector.
     A video has one frame for every 4 seconds it starts, 8 at most: 1 under 4 s
     (a live photo is one picture, its middle frame), 2 under 8 s, 3 under 12 s,
     and 8 from 28 s. FFmpeg scales each frame down to 512 pixels itself.
     If no frame can be extracted the video thumbnail is used instead.
4. A search embeds the query text on the server, then scans the library's vectors.

A pass reads the images of the next 2 medias while the model is on the current one
(`media_embedding_images`, then `embed_media_images`). Reading is everything that
is not the model: the thumbnail from storage, FFmpeg for the frames, decoding,
scaling down to 512 pixels. The medias are still embedded and stored one by one, in
the order of the queue. A pause or a stop still takes effect once the media in
progress is done; what was read ahead is dropped.

Frames are read several at a time: one for every three threads of the machine, 4 at
most, whatever the number of videos being read. Each frame is an FFmpeg process
decoding a video, so a machine with 4 threads still reads them one after the other.

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

On a Ryzen 5 5600X (6 cores, 12 threads) with an RTX 3090, on 15 photos and 11
videos of a real library (phone videos, 1080p and more), per media:

| | Photo | Video: reading its frames | Video: the model |
| --- | --- | --- | --- |
| CPU, 4 threads | 2.3 s | 1.3 s | 17 s |
| GPU | 0.11 s | 1.4 s | 0.9 s |

Another pass was using 4 cores of the machine meanwhile, so an idle one does a
little better. Reading a photo, its thumbnail, takes 3 ms.

On the CPU the time is the vision encoder, 1.9 s of a photo, and in it the matrix
products of the q4 weights (`MatMulNBits`, 70%) and the attention (18%). With 6
threads instead of 4 the encoder takes 1.35 s. Sending the 8 frames of a video
through a graph in one batch was tried and gains nothing, on the CPU or the GPU.

Reading the frames of a video took 5.3 s when they were read one after the other,
each written by FFmpeg as a fully compressed PNG and encoded once more after being
scaled down.

With the model on the GPU, reading the frames is what a pass waits for: on a
stretch of 10 to 30 s videos in 1080p and 4K, FFmpeg kept the CPU at 70 to 90%
while the GPU was idle, about 6 s per video. That is why a video now has a frame
for every 4 seconds rather than always 8, and why FFmpeg scales the frame down
before it writes it: a 4K HDR frame is 35 MB as a PNG, and under 1 MB at 512 pixels.
The table above was measured before these two changes, with 8 frames at full size.

The text backbone stays loaded once used, so searches stay fast. The vision encoder
is loaded when a pass starts embedding and released when the pass ends. Only one
pass runs at a time. On the CPU a graph uses up to 4 threads;
`REDSEAT_EMBEDDINGS_THREADS` sets another number, for example the number of cores
of a machine that has nothing else to do.

## GPU

Images can go through the GPU instead of the CPU, with the WebGPU provider of ONNX
Runtime. It is a build option, off by default:

```bash
cargo build --release --features gpu
```

- **Libraries.** The build links another ONNX Runtime, which needs its libraries
  beside the binary to use the GPU. On Windows: `webgpu_dawn.dll`, `dxcompiler.dll`
  and `dxil.dll`. Cargo puts them in `target/release`; a packaged release has to
  ship them. When one is missing the server still starts, says so in the log when a
  pass begins, and embeds on the CPU. For `webgpu_dawn.dll` that takes two things:
  `build.rs` has Windows load it on first use instead of at startup, and
  `gpu_library_present` in `gemma.rs` loads it before the GPU is touched. On Linux
  and macOS the library is loaded with the program, which does not start without it.
- **Platforms.** Only tried on Windows x86_64 with an NVIDIA card (Direct3D 12).
  `ort` also has this runtime for Linux x86_64 and macOS arm64; a target without
  one builds the usual runtime and embeds on the CPU.
- **What runs where.** A text query always runs on the CPU, so a search does not
  wait for a pass. During a pass the vision encoder and a second copy of the
  backbone are on the GPU, about 0.6 GB of its memory, released when the pass ends.
- **When the GPU cannot be used.** The log says which of the two is used when a
  pass starts. A GPU that cannot be loaded leaves the pass on the CPU. A GPU that
  fails on an image has that image embedded on the CPU, like the next ones until
  the pass ends; the following pass tries the GPU again. None of this is charged to
  a media.
- **Staying on the CPU.** `REDSEAT_EMBEDDINGS_CPU=true` keeps a `gpu` build on the
  CPU. It is read when the model is loaded, so it needs a restart.

### Same vectors, same id

The GPU runs the same graphs on the same q4 weights, in float like the CPU, so its
vectors carry the same embedder id. The two differ by rounding only. 26 medias that
a CPU pass had already embedded (15 photos, 11 videos) were embedded again and
compared with their stored vectors:

| | Identical to the stored vector | The others |
| --- | --- | --- |
| CPU | 26 of 26 | |
| GPU | 25 of 26 | 2 of 768 values, one step each |

The ignored test `embeddinggemma_gpu_matches_cpu` in `src/tools/embedding/gemma.rs`
embeds the images of `test_data` on both and fails past one step, or past 1% of the
values:

```bash
cargo test --features gpu --bin redseat-rust embeddinggemma_gpu_matches_cpu -- --ignored --nocapture
```

A debug build does not give the vectors of a release build: on the same 26 medias,
up to 28 of 768 values were one step away, on the CPU as on the GPU. Compare
vectors made by the same profile.

This comparison was made when a video was 8 frames scaled down by the server. A
video embedded since no longer gives the vector stored before, see
[Known limits](#known-limits); a photo still does.

### Why not DirectML

The ONNX Runtime of the default Windows build already has DirectML, with no library
to ship. It was tried and left aside: it refuses the vision encoder as it is
(`Reshape` with `allowzero`), and with that attribute cleared it runs the encoder in
0.2 to 0.4 s, 3 to 6 times slower than WebGPU, because its `Gelu` stays on the CPU
and the data goes back and forth at every layer.

## The embedder abstraction

```rust
pub trait Embedder: Send + Sync {
    fn spec(&self) -> &EmbedderSpec;          // id, dimension, search_dimension, quantization_scale, text_cutoff
    fn embed_decoded_image(&self, image: &image::DynamicImage) -> EmbeddingResult<Vec<f32>>;
    fn embed_image(&self, bytes: &[u8]) -> EmbeddingResult<Vec<f32>> { .. }  // decodes, then the one above
    fn embed_text(&self, query: &str) -> EmbeddingResult<Vec<f32>>;
    fn prepare_image_resources(&self) -> EmbeddingResult<()> { Ok(()) }
    fn release_image_resources(&self) {}
}
```

All three return L2-normalised float vectors in the same space. Storage, the
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
the int8 scale below. Where the graphs run does not: see
[Same vectors, same id](#same-vectors-same-id). After an id change the old rows are
simply ignored and every media becomes pending again; old rows can be deleted with
`DELETE FROM media_embeddings WHERE embedder = '<old id>'`.

### Adding another embedder

1. Implement `Embedder` in a new file under `src/tools/embedding/`, with a `SPEC`
   constant. Set `search_dimension` equal to `dimension` unless the model is trained
   so that a prefix of the vector is usable on its own.
2. Derive its `quantization_scale` and its `text_cutoff` (see below) and hard-code
   them next to the id.
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
offered again by later passes, at most once per pass and after the medias that
never failed, and is given up on after 3 attempts. A success deletes the row. `failed` in the status below counts the
medias given up on; `POST .../embeddings/process?retryFailed=true` forgets the
failures of the library.

Two guards keep a problem that is not about the medias from being charged to them:

- A pass loads the whole model, vision encoder included, before it touches a media.
  A model that cannot load stops the pass with no failure recorded.
- After 5 failures in a row a pass leaves the library until the next one. A broken
  model or unreachable storage then costs those 5 medias an attempt, not the whole
  library.

Two things keep that second guard from holding the queue on medias that are just
unreadable. Without them a pass tried 5 of them, left, and the next one started on
the same 5: 30 such files in a row took 3 hours, three passes each.

- The medias that already failed are offered last, after those never tried. Their
  retries can end a pass, never start it.
- A media whose file is recognised as an image and cannot be decoded is not counted
  in the 5 (`is_undecodable_media`): that failure is about the media. A read that
  gives nothing recognisable is still counted, since the storage may be the cause,
  and so is any other error.

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
on the first search of the library, a text on its listing included, updated as
vectors are added and medias deleted, and dropped with the library. A search is a plain linear scan, about
21 ms per 100 000 medias on one thread; there is no approximate index.

### Cut-off

A text search leaves out the medias scoring under the `text_cutoff` of the embedder.
For `embeddinggemma-2/q4/t140`, **`text_cutoff = 0.67`**. It is a search setting
only: changing it does not need a new id.

It was chosen on 400 COCO val2017 photos and their 2 001 captions, scored as
production does (int8, first 512 values):

| Score of a text with | 1st percentile | 5th | median | 95th | 99th |
| --- | --- | --- | --- | --- | --- |
| the photo it describes | 0.664 | 0.699 | 0.759 | 0.806 | 0.828 |
| another photo | 0.474 | 0.496 | 0.552 | 0.626 | 0.679 |

At 0.67, 99% of the described photos are kept and 1.3% of the others, many of
which show the same kind of scene. The ignored test `embeddinggemma_text_cutoff` in
`src/tools/embedding/gemma.rs` prints these shares for a range of cut-offs:

```bash
REDSEAT_EMBEDDING_CALIBRATION_DIR=/path/to/images \
REDSEAT_EMBEDDING_CAPTIONS=/path/to/captions.json \
  cargo test --bin redseat-rust embeddinggemma_text_cutoff -- --ignored --nocapture
```

`captions.json` is an array of `{ "file": "000000397133.jpg", "captions": ["..."] }`.

Other queries on the same photos, where a photo counts as relevant when at least
two of its captions hold the word:

| Query | Relevant photos kept at 0.67 |
| --- | --- |
| A caption | 99% |
| One English word ("dog", "kitchen"; 38 words) | 88% |
| One or two French words ("un chien", "salle de bain"; 15 queries) | 55% |

Short and non-English queries score lower, so they lose more: at 0.66 the last two
lines are 93% and 70%. What sits just under the cut-off is mostly near misses
("pizza" and a pie, "toilet" and a bathroom sink), and under 0.65 almost nothing
is related.

A query about something no photo shows (30 queries such as "a submarine") returns
nothing 93% of the time, and a meaningless one ("asdfgh", "IMG_2034") 90% of the
time. Without a cut-off both return the whole library.

Subtracting a per-query baseline (the score of the text with an average photo, or
the mean score over the library) was tried and separates matches no better than the
raw score, so the cut-off is a plain constant.

## Turning it on

Embeddings are off by default. Set `embeddings` in the library settings:

```
PATCH /libraries/{libraryId}
{ "settings": { "embeddings": true } }
```

The other settings are kept: see `LIBRARIES.md`.

Turning it off stops the background work once the media in progress is done; a
pass that was running reports `stopped`. Existing vectors are kept and stay
searchable.

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
  The two endpoints change only that setting and keep the others, and they are the
  only way to change it: `embeddingsPaused` is ignored in a library update.

Libraries with client-side encryption (`crypt`) are skipped: the server cannot read
their files. Password-encrypted libraries work, the server decrypts them as it does
for thumbnails.

## API

All routes are under `/libraries/{libraryId}/medias`.

### `GET /embeddings/search?text=...&limit=50&offset=0&minScore=...`

Medias matching a text query, best first. `limit` defaults to 50 (maximum 500).
A page may reach at most 10 000 results down the ranking: a request with
`offset + limit` above that is refused.

Medias scoring under `minScore` are left out. It defaults to the cut-off of the
embedder (0.67, see [Cut-off](#cut-off)); `minScore=0` returns the whole ranking.

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
| `ok` | `results` is the ranked list; empty when nothing reaches the minimum score |
| `noEmbeddings` | Nothing is embedded yet in this library for the active embedder |
| `modelDownloading` | The model is being downloaded; retry later |
| `mediaNotEmbedded` | (`similar` only) the reference media has no vector yet |

When `status` is not `ok`, `results` is empty and `message` explains why.

### `GET /?text=...`: the media listing

The listing itself searches by content when its query has a `text` and the library
has embedded medias. The answer keeps its shape, an array of medias. What changes
is its order, and that it goes on to the medias whose content matches the text:

1. First the medias the listing returns for this query without embeddings: the
   name or the description contains the text, and every other criterion holds.
   They are ordered closest content first. Those with no vector come last in this
   group, in the order given by `sort` and `order`.
2. Then the other medias that meet every criterion but the text and whose content
   scores at least the cut-off of the embedder (see [Cut-off](#cut-off)), closest
   first. Medias with no vector are left out of this group.

The other criteria (`tags`, `people`, `types`, dates, ...) and the access rules of
the user decide what may be returned in both groups. `sort` and `order` have no
other effect than the one given above: a text search is ordered by relevance.

`limit` (default 200) and `pageKey` page through this ranking. Only the id after
the `|` of `pageKey` is read, and the next page starts after that media, so a
client that sends `<sort value>|<id of the last media>` pages as it does on the
plain listing. A page is empty past the end of the ranking, and also when the
media named is no longer in it (deleted, or it no longer meets the criteria).

`GET /count` is not changed: with a `text` it counts the first group.

The listing answers as it does without embeddings, with the text as a filter and
the order of `sort`, when:

- the query has no `text`;
- no media of the library has a vector for the active embedder;
- the model is not on disk yet (its download is started), or it fails on the text;
- `pageKey` holds no id (the single-number form).

A text search costs one text embedding, two queries for ids and one similarity per
media the criteria allow, whatever the page.

### `GET /{mediaId}/similar?limit=50&offset=0&minScore=...`

Medias closest to the given one, which is left out of the results. Same response.
Uses stored vectors only, so it works even when the model is not loaded.

There is no cut-off by default: every other embedded media is ranked. Two images
score higher together than a text and an image do, so the text cut-off does not
apply here; `minScore` sets one for the request.

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
media), then exactly one of:

- `completed`: nothing is left to embed in this pass.
- `paused`: the library was paused.
- `stopped`: embeddings were turned off for the library, or it was removed.
- `interrupted`: left after repeated failures; a later pass carries on.
- `failed`: the pass could not run, for example when the model cannot be downloaded
  or loaded. The reason is in the server log.

A pass that announced its start always sends one of these five. See `docs/SSE.md`.

## Known limits

- **Scores are compressed**, so the cut-off sits in a narrow band: a match scores
  around 0.70 to 0.80 and unrelated content around 0.50 to 0.63. It trades the two
  errors measured in [Cut-off](#cut-off): some loosely related medias pass, and
  short or non-English queries lose part of their matches.
- **Videos pass the cut-off too easily.** On a real library a video was about 20
  times more likely than a photo to pass it for an unrelated text (1.3% against
  0.06%). The likely cause is the averaging of 8 frames: re-normalising the average
  raises its score with every text. Fixing it changes the stored vectors, so it
  needs a new embedder id.
- **No content filtering.** A neutral query can surface adult content if the
  library contains it. Nothing flags or filters it yet.
- **One vector per video**, from 8 frames at most. A short scene in a long video is
  easily missed.
- **Videos embedded before 13 October 2026 keep their vector**, made of 8 frames
  whatever their length, each scaled down by the server. Since then a video has a
  frame for every 4 seconds and FFmpeg scales it, which gives a slightly different
  picture (2 of 255 per value on a test pattern). The embedder id did not change
  for it: the vectors stay comparable. To have these videos embedded again, with
  the server stopped:
  `DELETE FROM media_embeddings WHERE media_ref IN (SELECT id FROM medias WHERE type = 'video')`.
- **Vectors are not refreshed** when a media's image is replaced. Delete its row
  in `media_embeddings` to have it embedded again.
- **Photos are embedded from the server's thumbnail-sized image**, so very small
  details of a large photo are lost.
- **Slow on the CPU**: 2 to 3 s per image, so a large library takes days to
  backfill. A GPU build takes about 0.1 s per image, see [GPU](#gpu).
- **No Intel macOS build.** The model needs ONNX Runtime 1.23 or later, and `ort`
  ships no prebuilt runtime for `x86_64-apple-darwin` at those versions.
