# Tools layer

Low-level media processing, inference, encryption, and scheduling. Root guidance
applies; paths below are relative to this directory.

## Entry points

- `recognition.rs`: ONNX face detection, alignment, embeddings, and model loading.
- `video_tools.rs`: FFmpeg/FFprobe provisioning, `VideoCommandBuilder`, and progress.
- `video_tools/ytdl.rs`: YT-DLP integration.
- `image_tools.rs`, `image_tools/`, `convert/`: image handling and format converters.
- `encryption.rs`: streaming encryption/decryption and the stored file format.
- `scheduler/mod.rs`: task lifecycle and cancellation; sibling modules implement jobs.

## Change constraints

- Keep blocking inference off async executor threads using the existing async
  wrappers. ONNX sessions use mutexes and one intra-op thread to limit CPU
  contention; account for both when changing scan concurrency.
- Use existing external-tool wrappers and preserve platform handling, progress
  reporting, and error propagation. `FFMPEG_LOCK` coordinates binary use and
  updates; check its read/write scope when changing provisioning or execution.
- Preserve compatibility with existing encrypted files. Read the header and
  reader/writer implementations before changing offsets, size calculations,
  buffering, or padding; validate round trips and boundary sizes for such changes.
- In `convert/heic.rs`, preserve ownership and cleanup of libheif allocations
  across success and error paths.
- Preserve scheduler cancellation and running-task bookkeeping when changing
  task execution. Keep batch sizes and cadence tied to the implementation rather
  than copying current values into instructions.
