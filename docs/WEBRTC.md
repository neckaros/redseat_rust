# WebRTC API transport

RedSeat accepts DataChannel protocol v1 on reliable, unordered channels named
`redseat-api-v1`. The cloud connection is used only for signaling. API requests,
uploads, downloads, and events travel directly between the client and server;
the server does not use TURN candidates or send application payloads through the
cloud.

Each wire request is converted to an in-process Axum request and executed by the
same router used by HTTP. This preserves the normal authentication middleware,
route guards, controller permissions, query handling, range middleware, status
codes, headers, and response bodies. The cloud signaling user ID is retained only
as peer-session metadata and never authenticates an API request.

## Streaming and bounds

- Complete DataChannel messages are limited to 16 KiB.
- A peer may have at most 64 active requests, 16 active subscriptions, and 128
  incomplete or orphan payloads.
- Request bodies are dispatched as soon as their descriptor arrives. In-order
  chunks flow into Axum through a bounded 16-chunk queue; only chunks that arrive
  ahead of the next expected chunk are temporarily spooled. This allows large
  uploads to reach the existing handlers incrementally instead of staging the
  complete file first. A per-payload capacity waiter resumes a full body queue;
  the peer control loop never waits for an upload handler to consume bytes.
- Chunks may precede their control frame and may arrive out of order; duplicate,
  conflicting, incomplete, or malformed framing is rejected. Each payload may
  hold at most 4,096 chunks ahead of its consumer. Reorder files use reusable
  16-KiB slots and truncate released tail slots, so quota reflects the active
  reorder window rather than total upload size. Spooling is limited to 512 MiB
  per peer and 8 GiB across the server.
- A request with `responseType: "stream"` opts into incremental response framing
  by setting `responseStream: 1`. The server sends `response-start`, bounded
  one-MiB `response-segment` payloads, and `response-end`. A declared length is
  optional, so unknown-length playback starts before EOF. Segment descriptors
  remain ordinary v1 payloads and therefore retain unordered-channel validation.
  The browser grants `response-credit` from its `ReadableStream` pull callback;
  the server retains at most two credits, bounding memory when playback is slow.
- Legacy finite responses remain capped at 256 MiB. An oversized response receives
  a request-scoped 413 transport error rather than closing the shared channel.
  Unknown-length legacy responses use the same aggregate spool budgets.
- HEAD, informational, 204, and 304 responses preserve HTTP headers (including
  `Content-Length`) while advertising an empty wire payload.
- DataChannel sends pause above a 1 MiB buffered amount and resume after it drains
  to 256 KiB or lower.
- Incomplete payloads expire after 60 seconds. Channel failure, peer failure,
  cancellation, and server shutdown cancel active work and release spool files.
- The bounded pre-dispatch DataChannel queue applies cancellation-aware
  backpressure when full instead of disconnecting the peer. Bounded cancellation
  tombstones prevent a request or subscription delivered after its cancel frame
  from starting on the unordered channel.

The transport reconstructs the v1 form-data envelope as a streaming multipart
request so existing upload handlers continue to receive ordered entries, repeated
names, filenames, content types, and binary bytes. It also consumes the response
from existing SSE routes, translates SSE fields to v1 event frames, and preserves
per-subscription sequence order.

Cancellation stops transport processing and drops the in-process request future;
it does not undo mutations that an existing handler has already committed. The
server never retries or replays requests after reconnection.
