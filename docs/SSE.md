# Server-Sent Events (SSE) API

This document describes the SSE endpoint for receiving real-time events from the server. SSE provides an alternative to Socket.IO for clients that prefer a simpler, HTTP-native approach.

## Overview

- **Endpoint**: `GET /sse`
- **Authentication**: Requires valid authentication (same as other API endpoints)
- **Protocol**: Standard SSE (EventSource API)
- **Direction**: Server-to-client only (one-way)

Both SSE and Socket.IO broadcast the same events. Choose SSE when you need:
- Simpler client implementation
- HTTP/2 multiplexing benefits
- Native browser EventSource support
- One-way server push only

## Query Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `libraries` | string (optional) | Comma-separated list of library IDs to filter events |

Example: `/sse?libraries=lib1,lib2` will only receive events for those libraries.

## Event Types

| Event Name | Description | Required Permission |
|------------|-------------|---------------------|
| `heartbeat` | Transport keepalive emitted every 30 seconds; clients should ignore its `{}` payload | None beyond SSE authentication |
| `library` | Library created/updated/deleted | Library read access |
| `library-status` | Library status changes | Library admin |
| `medias` | Media items created/updated/deleted | Library read access |
| `upload_progress` | Upload progress | Library read access |
| `convert_progress` | Video conversion progress | Library read access |
| `episodes` | Episodes created/updated/deleted | Library read access |
| `series` | Series created/updated/deleted | Library read access |
| `movies` | Movies created/updated/deleted | Library read access |
| `books` | Books created/updated/deleted | Library read access |
| `people` | People created/updated/deleted | Library read access |
| `tags` | Tags created/updated/deleted | Library read access |
| `backups` | Backup job events | Library admin or server admin |
| `backups-files` | Backup file progress | Server admin only |
| `media_progress` | Playback position tracking | User-specific (only progress owner) |
| `media_rating` | Rating changes (media, movies, series, episodes, books, people) | User-specific (only rating owner) |
| `watched` | Content marked as watched | User-specific (only watched owner) |
| `unwatched` | Content unmarked as watched | User-specific (only watched owner) |
| `request_processing` | Request processing status updates | Library read access |

The endpoint sends `Cache-Control: no-cache, no-transform` and
`X-Accel-Buffering: no` so reverse proxies flush heartbeat and data events
immediately instead of buffering an otherwise idle stream.

When an authorized server administrator connects or reconnects, the server first
emits an `updated` `backups` event for every configured backup with its current
in-memory status. This snapshot clears stale `inProgress` client state after a
server restart or a missed terminal event. Live events that occur while the
snapshot is assembled are queued; backup updates are coalesced into the snapshot
so it cannot be followed by older backup state, and other events are delivered
afterward.

## Entity search streams

The following entity-scoped endpoints stream source lookup results as SSE:

- `GET /libraries/{libraryId}/books/{bookId}/searchstream`
- `GET /libraries/{libraryId}/movies/{movieId}/searchstream`
- `GET /libraries/{libraryId}/series/{serieId}/seasons/{season}/searchstream`
- `GET /libraries/{libraryId}/series/{serieId}/seasons/{season}/episodes/{number}/searchstream`

All entity lookup endpoints, including their non-streaming `/search` variants,
accept these optional query parameters:

- `name`: a non-empty title/name override to use for the plugin lookup instead of
  the entity's stored title/name.
- `source`: a plugin/source path or stored plugin ID (or a comma-separated list
  when requesting the first page) to query.
- `pageKey`: an opaque provider-specific page key passed through to the selected
  plugin. A request with `pageKey` must identify exactly one non-empty `source`,
  because page keys are not portable across providers. Non-blank page keys are
  preserved verbatim, including leading or trailing whitespace.

For example, the next page of movie results from one lookup plugin can be
requested with:

```text
GET /libraries/{libraryId}/movies/{movieId}/searchstream?name={alternateTitle}&source={pluginId}&pageKey={pageKey}
```

The server does not impose the current 25-result page size. Each plugin decides
its page size and how its page key is encoded; plugins that ignore `pageKey`
remain single-page sources. A non-streaming `/search` response is grouped by
source so every provider can return its own continuation cursor:

```typescript
interface LookupSourceResults {
  sourceId: string;
  sourceName: string;
  results: RsGroupDownload[];
  nextPageKey?: string;
}

type LookupResponse = LookupSourceResults[];
```

`sourceId` is the unique stored plugin ID, so it identifies one installation
even when multiple installed plugins use the same WASM path.

Lookup plugins can advertise another page by wrapping their existing result:

```json
{
  "result": { "requests": [] },
  "nextPageKey": "provider-defined-opaque-cursor"
}
```

The `result` value uses the existing `RsLookupSourceResult` representation, so
`groupRequest`, `notFound`, and `notApplicable` remain valid alternatives.
Unwrapped legacy plugin responses are still accepted, but cannot advertise a
continuation cursor.

Each source emits a `results` event with both the legacy flattened requests and
the complete grouped downloads:

```typescript
interface LookupSearchEvent {
  sourceId: string;
  sourceName: string;
  // Backward-compatible flattened view: one entry per request.
  results: Array<{
    request: RsRequest;
    matchType?: RsLookupMatchType;
  }>;
  // Group-aware view: preserves provider grouping and group metadata.
  downloads: RsGroupDownload[];
  nextPageKey?: string;
}

interface RsGroupDownload {
  group: boolean;
  groupThumbnailUrl?: string;
  groupFilename?: string;
  groupMime?: string;
  requests: RsRequest[];
  infos?: MediaForUpdate;
  matchType?: RsLookupMatchType;
}
```

Existing clients can continue consuming `results` unchanged. Group-aware
clients should consume `downloads` instead and must not render both views. A
provider result that is not grouped is still represented in `downloads` with
`group: false`. The server adds the searched book, movie, series, season, and
episode association to each download before emitting it, so a grouped download
can be posted to `POST /libraries/{libraryId}/medias/download` without losing
its entity relationship.

`library-status` is also used for async library deletion lifecycle updates. Current messages include:
- `delete-started`
- `delete-removing-tracked-media`
- `delete-media-progress:{current}/{total}`
- `delete-cleaning-local-cache`
- `delete-cleaning-database-files`
- `delete-completed`
- `delete-failed: ...`

Password-encryption maintenance emits these messages while the library's file
operations are temporarily unavailable:

- `encryption-running`
- `encryption-retry: ...`
- `encryption-failed: ...`
- `encryption-completed`

## TypeScript Client Examples

### Basic Connection

```typescript
const eventSource = new EventSource('/sse', {
  // Include credentials if using cookies for auth
  withCredentials: true
});

// Or with token-based auth (depends on your auth setup)
// You may need to pass the token via query param or use fetch-event-source library
const eventSource = new EventSource('/sse?token=' + authToken);

eventSource.onopen = () => {
  console.log('SSE connection established');
};

eventSource.onerror = (error) => {
  console.error('SSE error:', error);
  // EventSource will automatically reconnect
};
```

### Type Definitions

```typescript
// Base action type for CRUD events
type ElementAction = 'Deleted' | 'Added' | 'Updated';

// Library events
interface LibraryMessage {
  action: ElementAction;
  library: ServerLibrary;
}

interface LibraryStatusMessage {
  message: string;
  library: string;
  progress?: number;
}

// Media events
interface MediasMessage {
  library: string;
  medias: MediaWithAction[];
}

interface MediaWithAction {
  action: ElementAction;
  // All Media fields are present at the top level (flattened), plus optional relations
  media: Media & { relations?: Relations };
}

interface Relations {
  people?: MediaItemReference[];
  peopleDetails?: Person[];
  tags?: MediaItemReference[];
  tagsDetails?: Tag[];
  series?: FileEpisode[];
  seriesDetails?: Serie[];
  movies?: string[];
  moviesDetails?: Movie[];
  books?: string[];
  booksDetails?: Book[];
}

type RsProgressType =
  | 'download'
  | 'transfert'
  | 'analysing'
  | 'finished'
  | { duplicate: string }
  | { failed: string };

interface RsProgress {
  id: string;
  total?: number;
  current?: number;
  filename?: string;
  type: RsProgressType;
}

interface UploadProgressMessage {
  library: string;
  progress: RsProgress;
  remainingSecondes?: number;
}

interface ConvertProgress {
  id: string;
  filename: string;
  convertedId?: string | null;
  done: boolean;
  percent: number;
  status: string; // queued | pending | downloading | processing | completed | failed | canceled
  remainingSecondes?: number | null;
  request?: VideoConvertRequest | null;
}

interface ConvertMessage {
  library: string;
  progress: ConvertProgress;
}

// Content events
interface EpisodesMessage {
  library: string;
  episodes: EpisodeWithAction[];
}

interface SeriesMessage {
  library: string;
  series: SerieWithAction[];
}

interface MoviesMessage {
  library: string;
  movies: MovieWithAction[];
}

interface BooksMessage {
  library: string;
  books: BookWithAction[];
}

interface PeopleMessage {
  library: string;
  people: PersonWithAction[];
}

interface TagMessage {
  library: string;
  tags: TagWithAction[];
}

// Backup events
interface BackupMessage {
  backup: BackupWithStatus;
}

interface BackupFileProgress {
  library?: string;
  file: string;
  progress: number;
}

// Media progress (user-specific)
interface MediaProgress {
  userRef: string;
  mediaRef: string;
  progress: number;
  modified: number;
}

interface MediasProgressMessage {
  library: string;
  progress: MediaProgress;
}

// Rating events (user-specific)
// Supports rating media, movies, series, episodes, and books.
// The `type` field indicates the entity type being rated.
// For episodes, `refId` uses the format "serieRef:season:number".
interface Rating {
  type: string;   // ElementType: "media", "movie", "serie", "episode", "book", "person"
  refId: string;  // Reference ID of the rated entity
  userRef: string;
  rating: number;
  modified: number;
}

interface MediasRatingMessage {
  library: string;
  rating: Rating;
}

// Watched events (user-specific). IDs include the media type and identity scheme.
interface Watched {
  type: string;  // MediaType: "movie", "episode", etc.
  id: string;    // e.g. "movie:imdb/tt1234567" or "episode:imdb/tt0108778/1/2"
  userRef?: string;
  date: number;  // Timestamp when content was watched
  modified: number;
}

// Unwatched events (user-specific)
// NOTE: Different structure from Watched because the delete API accepts multiple IDs.
interface Unwatched {
  type: string;     // MediaType: "movie", "episode", etc.
  ids: string[];    // History IDs marked as deleted
  userRef?: string;
  modified: number;
}

// Request processing events
interface RequestProcessingMessage {
  library: string;
  processings: RequestProcessingWithAction[];
}

interface RequestProcessingWithAction {
  action: ElementAction;
  processing: RsRequestProcessing;
}

interface RsRequestProcessing {
  id: string;           // Internal nanoid for this processing record
  processingId: string; // Plugin's processing ID
  pluginId: string;     // ID of the plugin handling this request
  progress: number;     // 0-100 progress percentage
  status: string;       // "pending", "processing", "paused", "finished", "error"
  error?: string;       // Error message if status is "error"
  eta?: number;         // UTC timestamp (ms) for estimated completion
  mediaRef?: string;    // Optional reference to the media this processing is for
  originalRequest?: RsRequest; // The original request that started this processing
  modified: number;     // Last modified timestamp
  added: number;        // Creation timestamp
}

// Wrapper type matching the SSE event structure
type SseEvent =
  | { Library: LibraryMessage }
  | { LibraryStatus: LibraryStatusMessage }
  | { Medias: MediasMessage }
  | { UploadProgress: UploadProgressMessage }
  | { ConvertProgress: ConvertMessage }
  | { Episodes: EpisodesMessage }
  | { Series: SeriesMessage }
  | { Movies: MoviesMessage }
  | { Books: BooksMessage }
  | { People: PeopleMessage }
  | { Tags: TagMessage }
  | { Backups: BackupMessage }
  | { BackupsFiles: BackupFileProgress }
  | { MediaProgress: MediasProgressMessage }
  | { MediaRating: MediasRatingMessage }
  | { Watched: Watched }
  | { Unwatched: Unwatched }   // Note: different structure than Watched
  | { RequestProcessing: RequestProcessingMessage };
```

### Listening to Events

```typescript
// Listen to specific event types
eventSource.addEventListener('medias', (event) => {
  const data: SseEvent = JSON.parse(event.data);
  if ('Medias' in data) {
    const message = data.Medias;
    console.log(`Library ${message.library} medias updated:`, message.medias);

    message.medias.forEach(({ action, media }) => {
      switch (action) {
        case 'Added':
          console.log('New media:', media.id);
          break;
        case 'Updated':
          console.log('Updated media:', media.id);
          break;
        case 'Deleted':
          console.log('Deleted media:', media.id);
          break;
      }
    });
  }
});

eventSource.addEventListener('library-status', (event) => {
  const data: SseEvent = JSON.parse(event.data);
  if ('LibraryStatus' in data) {
    const { library, message, progress } = data.LibraryStatus;
    console.log(`Library ${library}: ${message} (${progress}%)`);
  }
});

eventSource.addEventListener('convert_progress', (event) => {
  const data: SseEvent = JSON.parse(event.data);
  if ('ConvertProgress' in data) {
    const { library, progress } = data.ConvertProgress;
    const eta = progress.remainingSecondes ? `, ~${progress.remainingSecondes}s remaining` : '';
    console.log(`Converting in ${library}: ${progress.filename} ${(progress.percent * 100).toFixed(2)}% - ${progress.status}${eta}`);
  }
});

eventSource.addEventListener('media_progress', (event) => {
  const data: SseEvent = JSON.parse(event.data);
  if ('MediaProgress' in data) {
    const { library, progress } = data.MediaProgress;
    console.log(`Progress update in ${library}: ${progress.mediaRef} at ${progress.progress}ms`);
  }
});

eventSource.addEventListener('watched', (event) => {
  const data: SseEvent = JSON.parse(event.data);
  if ('Watched' in data) {
    const watched = data.Watched;
    console.log(`Marked as watched: ${watched.type} ${watched.id} on ${new Date(watched.date)}`);
  }
});

eventSource.addEventListener('unwatched', (event) => {
  const data: SseEvent = JSON.parse(event.data);
  if ('Unwatched' in data) {
    const unwatched = data.Unwatched;
    // Unwatched events contain the history IDs marked as deleted
    console.log(`Unmarked as watched: ${unwatched.type} with IDs: ${unwatched.ids.join(', ')}`);
  }
});
```

### Library Filtering

```typescript
// Only receive events for specific libraries
const libraries = ['photo-library', 'video-library'];
const eventSource = new EventSource(`/sse?libraries=${libraries.join(',')}`);

eventSource.addEventListener('medias', (event) => {
  // Will only receive medias events for the specified libraries
  const data: SseEvent = JSON.parse(event.data);
  // ...
});
```

### Reconnection Handling

```typescript
class SseClient {
  private eventSource: EventSource | null = null;
  private reconnectAttempts = 0;
  private maxReconnectAttempts = 5;
  private baseDelay = 1000;

  connect(url: string) {
    this.eventSource = new EventSource(url);

    this.eventSource.onopen = () => {
      console.log('Connected');
      this.reconnectAttempts = 0;
    };

    this.eventSource.onerror = (error) => {
      console.error('Connection error:', error);

      // EventSource auto-reconnects, but you can add custom logic
      if (this.eventSource?.readyState === EventSource.CLOSED) {
        this.handleReconnect(url);
      }
    };

    // Add your event listeners
    this.setupEventListeners();
  }

  private handleReconnect(url: string) {
    if (this.reconnectAttempts >= this.maxReconnectAttempts) {
      console.error('Max reconnection attempts reached');
      return;
    }

    const delay = this.baseDelay * Math.pow(2, this.reconnectAttempts);
    this.reconnectAttempts++;

    console.log(`Reconnecting in ${delay}ms (attempt ${this.reconnectAttempts})`);
    setTimeout(() => this.connect(url), delay);
  }

  private setupEventListeners() {
    if (!this.eventSource) return;

    const events = [
      'library', 'library-status', 'medias', 'upload_progress',
      'convert_progress', 'episodes', 'series', 'movies', 'books',
      'people', 'tags', 'backups', 'backups-files', 'media_progress',
      'media_rating', 'watched', 'unwatched', 'request_processing'
    ];

    events.forEach(eventName => {
      this.eventSource!.addEventListener(eventName, (event) => {
        this.handleEvent(eventName, JSON.parse(event.data));
      });
    });
  }

  private handleEvent(eventName: string, data: SseEvent) {
    // Dispatch to your application's event handlers
    console.log(`Received ${eventName}:`, data);
  }

  disconnect() {
    this.eventSource?.close();
    this.eventSource = null;
  }
}

// Usage
const client = new SseClient();
client.connect('/sse');
```

### React Hook Example

```typescript
import { useEffect, useState, useCallback } from 'react';

interface UseSseOptions {
  libraries?: string[];
  onEvent?: (eventName: string, data: SseEvent) => void;
}

function useSse(options: UseSseOptions = {}) {
  const [isConnected, setIsConnected] = useState(false);
  const [error, setError] = useState<Event | null>(null);

  useEffect(() => {
    const params = new URLSearchParams();
    if (options.libraries?.length) {
      params.set('libraries', options.libraries.join(','));
    }

    const url = `/sse${params.toString() ? '?' + params.toString() : ''}`;
    const eventSource = new EventSource(url);

    eventSource.onopen = () => {
      setIsConnected(true);
      setError(null);
    };

    eventSource.onerror = (err) => {
      setError(err);
      if (eventSource.readyState === EventSource.CLOSED) {
        setIsConnected(false);
      }
    };

    // Subscribe to all event types
    const eventTypes = [
      'library', 'library-status', 'medias', 'upload_progress',
      'convert_progress', 'episodes', 'series', 'movies', 'books',
      'people', 'tags', 'backups', 'backups-files', 'media_progress',
      'media_rating', 'watched', 'unwatched', 'request_processing'
    ];

    eventTypes.forEach(eventName => {
      eventSource.addEventListener(eventName, (event) => {
        const data = JSON.parse(event.data);
        options.onEvent?.(eventName, data);
      });
    });

    return () => {
      eventSource.close();
    };
  }, [options.libraries?.join(',')]);

  return { isConnected, error };
}

// Usage in a component
function MediaLibrary({ libraryId }: { libraryId: string }) {
  const [medias, setMedias] = useState<Media[]>([]);

  const handleEvent = useCallback((eventName: string, data: SseEvent) => {
    if (eventName === 'medias' && 'Medias' in data) {
      const { medias: updates } = data.Medias;
      setMedias(current => {
        // Apply updates to current state
        const updated = [...current];
        updates.forEach(({ action, media }) => {
          const index = updated.findIndex(m => m.id === media.id);
          if (action === 'Added' && index === -1) {
            updated.push(media);
          } else if (action === 'Updated' && index !== -1) {
            updated[index] = media;
          } else if (action === 'Deleted' && index !== -1) {
            updated.splice(index, 1);
          }
        });
        return updated;
      });
    }
  }, []);

  const { isConnected } = useSse({
    libraries: [libraryId],
    onEvent: handleEvent
  });

  return (
    <div>
      <span>Status: {isConnected ? 'Connected' : 'Disconnected'}</span>
      {/* Render medias */}
    </div>
  );
}
```

## Search Stream Endpoints

Search endpoints support SSE streaming so clients receive results progressively as each provider responds, instead of waiting for all providers to finish.

### Endpoints

| Endpoint | Query Parameters | Description |
|----------|-----------------|-------------|
| `GET /libraries/:libraryId/series/searchstream` | `name` (required), `ids`, `source`, `pageKey` (optional) | Stream series search results |
| `GET /libraries/:libraryId/movies/searchstream` | `name` (required), `ids`, `source`, `pageKey` (optional) | Stream movie search results |
| `GET /libraries/:libraryId/books/searchstream` | `name`, `author`, `isbn13` (at least one required); `source`, `pageKey` (optional) | Stream book search results; supplied fields are passed to plugins together |
| `GET /libraries/:libraryId/people/searchstream` | `name` (required), `ids`, `source`, `pageKey` (optional) | Stream people search results |

### How It Works

Each SSE event has event type `results` and contains one metadata plugin's
result page. `sourceId` is the value to send as `source` when requesting that
plugin's next page. If `nextPageKey` is present, send it back as `pageKey`.

### Event Format

Each `results` event contains one provider's results:

```json
{
  "sourceId": "tmdb.wasm",
  "sourceName": "TMDB",
  "results": [{"metadata": {"serie": { ... }}}],
  "nextPageKey": "2"
}
```

Then a second event for the next provider:

```json
{
  "sourceId": "anilist.wasm",
  "sourceName": "Anilist",
  "results": [{"metadata": {"serie": { ... }}}]
}
```

The `metadata` field is a tagged enum with one of: `serie`, `movie`, `book`, `episode`, `person`, `media`.

### TypeScript Example

```typescript
const params = new URLSearchParams({ name: 'one piece' });
const eventSource = new EventSource(
  `/libraries/${libraryId}/series/searchstream?${params}`
);

// Accumulate results and pagination cursors by provider.
const resultsByProvider: Record<string, SearchResult[]> = {};
const nextPageByProvider: Record<string, string> = {};

eventSource.addEventListener('results', (event) => {
  const data = JSON.parse(event.data);
  resultsByProvider[data.sourceId] = data.results;
  if (data.nextPageKey) {
    nextPageByProvider[data.sourceId] = data.nextPageKey;
  }
  renderResults(resultsByProvider);
});

eventSource.onerror = () => {
  // Stream finished or errored — close the connection
  eventSource.close();
};
```

To load another page, open a new request for only that provider:

```typescript
const nextParams = new URLSearchParams({
  name: 'one piece',
  source: 'tmdb.wasm',
  pageKey: nextPageByProvider['tmdb.wasm']
});
const nextPage = new EventSource(
  `/libraries/${libraryId}/series/searchstream?${nextParams}`
);
```

Page keys are opaque and provider-specific. A provider that omits
`nextPageKey` has no advertised next page.

### Non-Streaming Alternative

The same search is available as a regular JSON endpoint that returns all providers at once:

| Endpoint | Description |
|----------|-------------|
| `GET /libraries/:libraryId/series/search` | Returns all results grouped by provider |
| `GET /libraries/:libraryId/movies/search` | Returns all results grouped by provider |
| `GET /libraries/:libraryId/books/search` | Returns all results grouped by provider |
| `GET /libraries/:libraryId/people/search` | Returns all results grouped by provider |

Response format (an array with one entry per provider):

```json
[
  {
    "sourceId": "tmdb.wasm",
    "sourceName": "TMDB",
    "results": [{"metadata": {"movie": { ... }}}],
    "nextPageKey": "2"
  }
]
```

## Keepalive

The server sends a keepalive ping every 30 seconds to prevent connection timeouts. The ping is sent as a comment (`:ping`) which is ignored by the EventSource API.

## Error Handling

When a client falls behind and misses events (lag), the server will skip the missed events and continue with new ones. Consider implementing periodic full-sync if you need guaranteed delivery of all events.

## Watched/Unwatched Events

### Understanding the ID Format

History IDs include the media type so IDs from different domains cannot collide.

| Content | Format | Example |
|---------|--------|---------|
| Movie with IMDb ID | `movie:imdb/<imdbId>` | `movie:imdb/tt1234567` |
| Movie without IMDb ID | `movie:redseat/<movieId>` | `movie:redseat/abc123` |
| Series progress parent | `series:imdb/<seriesImdbId>` | `series:imdb/tt0108778` |
| Episode | `episode:imdb/<seriesImdbId>/<season>/<episode>` | `episode:imdb/tt0108778/1/2` |
| Episode fallback | `episode:redseat/<seriesId>/<season>/<episode>` | `episode:redseat/series123/1/2` |
| Book with ISBN-13 | `book:isbn13/<isbn13>` | `book:isbn13/9783161484100` |
| Book provider fallback | `book:<provider>/<providerId>` | `book:oleid/OL123M` |
| Series-backed installment | `book:<provider>/<providerId>\|volume:<volume>\|chapter:<chapter>` | `book:olwid/OL123W\|volume:2\|chapter:2.5` |
| Book local fallback | `book:redseat/<bookId>` | `book:redseat/book123` |

Episode IDs use the series IMDb ID and numeric season/episode tuple so watched state follows the same show across libraries. Series without an IMDb ID temporarily use the RedSeat fallback; if IMDb metadata is added later, the server migrates watched state and progress immediately.

Book IDs prefer ISBN-13, Open Library edition, Google Books volume, ASIN, Open Library work, another stable provider ID, and finally the RedSeat-local ID. Series-level providers include the volume/chapter tuple as ID details so separate installments do not share watched state. The watched row remains in per-user global history; library book rows do not store it. Book list and detail responses expose `watched` only as a request-user-specific projection hydrated from matching history aliases. When metadata enrichment changes the preferred ID, the server rewrites existing watched rows to the new ID without changing their per-user timestamps.

### REST API Endpoints

#### Mark as Watched

**Movies**: `POST /libraries/:libraryId/movies/:id/watched`
```json
{ "date": 1705766400000 }
```

**Episodes**: `POST /libraries/:libraryId/series/:serieId/seasons/:season/episodes/:number/watched`
```json
{ "date": 1705766400000 }
```

**Books**: `POST /libraries/:libraryId/books/:id/watched`
```json
{ "date": 1705766400000 }
```

**Direct History**: `POST /users/me/history`
```json
{
  "type": "movie",
  "id": "movie:imdb/tt1234567",
  "date": 1705766400000
}
```

For compatibility, raw movie and book provider IDs such as `imdb:tt1234567` and `isbn13:9783161484100` are normalized to their typed forms. Other media should send the typed history ID returned by the history API or SSE event.

#### Unmark as Watched (Remove from History)

**Movies**: `DELETE /libraries/:libraryId/movies/:id/watched`

**Episodes**: `DELETE /libraries/:libraryId/series/:serieId/seasons/:season/episodes/:number/watched`

**Books**: `DELETE /libraries/:libraryId/books/:id/watched`

**Direct History**: `DELETE /users/me/history`
```json
{
  "type": "movie",
  "ids": ["movie:imdb/tt1234567"]
}
```

The delete endpoint accepts an array so clients can delete more than one known history ID in one request.

### Example: Handling Watch State Changes

```typescript
// Track local watch state
const watchedItems = new Map<string, boolean>();

eventSource.addEventListener('watched', (event) => {
  const data = JSON.parse(event.data);
  if ('Watched' in data) {
    const { type, id, date } = data.Watched;
    console.log(`Marked as watched: ${type} ${id} on ${new Date(date)}`);
    watchedItems.set(id, true);
    // Update UI to show as watched
  }
});

eventSource.addEventListener('unwatched', (event) => {
  const data = JSON.parse(event.data);
  if ('Unwatched' in data) {
    const { type, ids } = data.Unwatched;
    console.log(`Unmarked as watched: ${type} with IDs: ${ids.join(', ')}`);
    // Remove all matching IDs from watched state
    ids.forEach(id => watchedItems.delete(id));
    // Update UI to show as unwatched
  }
});
```

### Matching SSE Events to Local Content

Match SSE events against the same typed history ID used by the REST API:

```typescript
interface LocalMovie {
  id: string;        // Local database ID
  imdb?: string;     // "tt1234567"
}

function isMatchingWatchedEvent(movie: LocalMovie, eventId: string): boolean {
  const historyId = movie.imdb
    ? `movie:imdb/${movie.imdb}`
    : `movie:redseat/${movie.id}`;
  return eventId === historyId;
}

// For Unwatched events (array of IDs)
function isMatchingUnwatchedEvent(movie: LocalMovie, eventIds: string[]): boolean {
  return eventIds.some(eventId => isMatchingWatchedEvent(movie, eventId));
}
```

## Offline Sync for Watch History

Unwatching content permanently deletes its history row. The server does not retain deletion tombstones.

### Client Sync Flow

```typescript
// Replace local watched state with a complete server snapshot.
async function syncHistory() {
  const response = await fetch('/users/me/history');
  const items: Watched[] = await response.json();
  replaceLocalWatched(items);
}
```

Connected clients can apply live `watched` and `unwatched` SSE events. After being offline, clients must perform this full reload because incremental history queries cannot report deletions.

### API Query Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `after` | number | Only return watched items modified after this timestamp (milliseconds). This does not report deletions. |
| `types` | string[] | Filter by content types (e.g., `movie`, `episode`) |
