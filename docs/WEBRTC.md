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

- Complete DataChannel messages are limited to 65,535 bytes in both directions:
  webrtc-rs reads each incoming message into a `u16::MAX` buffer and closes the
  channel on anything larger.
- A peer may have at most 128 active requests per channel (the browser client
  keeps at most 56 waiting for a response), 16 active subscriptions, and 128
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
  slots sized for the largest message, but quota is reserved for the bytes each
  slot actually holds (the files are sparse), so 16-KiB chunks cost 16 KiB and truncate released tail slots, so quota reflects the active
  reorder window rather than total upload size. Spooling is limited to 512 MiB
  per peer and 8 GiB across the server.
- A request with `responseType: "stream"` opts into incremental response framing
  by setting `responseStream: 1`. The server sends `response-start`, bounded
  one-MiB `response-segment` payloads, and `response-end`. A declared length is
  optional, so unknown-length playback starts before EOF. Segment descriptors
  remain ordinary v1 payloads and therefore retain unordered-channel validation.
  The browser grants `response-credit` from its `ReadableStream` pull callback,
  keeping several segments requested so the link stays busy across round trips.
  The server holds at most 16 unused credits per stream. A peer that exceeds that
  has lost track of its credits; the stream ends with a `response-end` error
  instead of the grant being silently dropped.
- Legacy finite responses remain capped at 256 MiB. An oversized response receives
  a request-scoped 413 transport error rather than closing the shared channel.
  Unknown-length legacy responses use the same aggregate spool budgets.
- HEAD, informational, 204, and 304 responses preserve HTTP headers (including
  `Content-Length`) while advertising an empty wire payload.
- A peer may open one extra reliable, **ordered** channel, `redseat-stream-v1`,
  alongside the API channel. It is served by its own dispatcher with the same
  protocol and shares the peer's spool budget; opening a second one closes that
  channel only. Clients use it for `stream` responses and large uploads. This
  separates the send queues before data leaves: each channel has its own
  application backpressure, and webrtc-sctp (which holds at most 128 KiB of
  unsent data) sends queued unordered API chunks before ordered stream chunks.
  Data already in flight shares the association's congestion window and the
  network path, so a stream that saturates the link still adds network queueing
  delay to API traffic.
- DataChannel sends pause above an 8 MiB buffered amount and resume after it drains
  to 4 MiB or lower. The buffered amount includes sent but unacknowledged bytes, so
  this bounds the data in flight per peer and therefore its throughput per round
  trip.
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
