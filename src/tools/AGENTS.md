# TOOLS LAYER - AGENTS.md

Low-level utilities: AI inference, video processing, encryption, scheduling.

## STRUCTURE

```
tools/
├── recognition.rs      # ONNX face detection/embedding (1505 lines)
├── video_tools.rs      # FFmpeg builder pattern (1276 lines)
├── encryption.rs       # AES-256-CBC streaming (551 lines)
├── image_tools.rs      # Native + ImageMagick hybrid (639 lines)
├── scheduler/          # Background task engine
│   ├── mod.rs          # RsScheduler: 15s tick loop
│   ├── backup.rs       # Full/incremental backups
│   ├── face_recognition.rs  # Chunked processing (50/batch)
│   └── refresh.rs      # Trakt/IMDb metadata sync
├── convert/            # Format converters (HEIC, JXL, RAW)
└── video_tools/ytdl.rs # YT-DLP integration
```

## KEY FILES

| File | Lines | Purpose |
|------|-------|---------|
| `recognition.rs` | 1505 | ONNX sessions (Detection/Alignment/Recognition), model auto-download |
| `video_tools.rs` | 1276 | `VideoCommandBuilder`, NVENC/libx264, progress parsing |
| `encryption.rs` | 551 | `AesTokioEncryptStream`, custom header with IV/thumbnail sizes |

## FACE RECOGNITION ENGINE

```rust
FaceRecognitionService {
    detection: Session,    // SCRFD model
    alignment: Session,    // Face alignment  
    recognition: Session,  // ArcFace embedding
}
```

- **Model Provisioning**: Auto-downloads from Hugging Face (Buffalo_L)
- **Math**: Manual similarity transform, bilinear interpolation for ArcFace alignment
- **Threading**: `intra_threads(1)` to prevent CPU thrashing

## VIDEO TOOLS

```rust
VideoCommandBuilder::new()
    .input(path)
    .set_video_codec(codec, quality)  // NVENC detection
    .add_overlay(logo_path)
    .output(dest)
    .execute_with_progress(callback)
```

- **Hardware Accel**: CUDA detection via `cuda_runtime_available`
- **Progress**: Parses `frame=` from FFmpeg stdout
- **Global Lock**: `FFMPEG_LOCK` protects binary path updates

## ENCRYPTION FORMAT

```
[16-byte IV][4-byte thumb_size][4-byte meta_size][32-byte mime][256-byte mime2][encrypted_data]
```

- AES-256-CBC with PKCS7 padding
- Streaming: `AesTokioEncryptStream` / `AesTokioDecryptStream`

## SCHEDULER

```rust
RsScheduler {
    items: Vec<RsSchedulerItem>,
    running: HashMap<String, CancellationToken>,
}
// Ticks every 15 seconds
```

| Task | Batch | Notes |
|------|-------|-------|
| BackupTask | Full library | Handles `.db` files + media |
| FaceRecognitionTask | 50 items | Detection + clustering |
| RefreshTask | Incremental | Tracks last update in `.txt` files |

## EXTERNAL BINARIES

| Binary | Auto-download | Sources |
|--------|---------------|---------|
| FFmpeg | Yes | BtbN (Win/Linux), evermeet.cx (macOS) |
| FFprobe | Yes | Same as FFmpeg |
| YT-DLP | Yes | GitHub releases |
| ONNX models | Yes | Hugging Face (Buffalo_L) |

## GOTCHAS

- `FFMPEG_LOCK` can stall parallel transcodes
- Manual vision math in `recognition.rs` - error-prone vs OpenCV
- First run requires internet for model downloads
- `unsafe` blocks in `convert/heic.rs` for libheif bindings
