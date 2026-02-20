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
| `media_rating` | Media rating changes | User-specific (only rating owner) |
| `watched` | Content marked as watched | User-specific (only watched owner) |
| `unwatched` | Content unmarked as watched | User-specific (only watched owner) |
| `request_processing` | Request processing status updates | Library read access |

`library-status` is also used for async library deletion lifecycle updates. Current messages include:
- `delete-started`
- `delete-removing-tracked-media`
- `delete-media-progress:{current}/{total}`
- `delete-cleaning-local-cache`
- `delete-cleaning-database-files`
- `delete-completed`
- `delete-failed: ...`

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

interface UploadProgressMessage {
  library: string;
  mediaId: string;
  progress: number;
  // ... additional fields
}

interface ConvertMessage {
  library: string;
  mediaId: string;
  progress: number;
  status: string;
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

// Media rating events (user-specific)
interface MediaRating {
  userRef: string;
  mediaRef: string;
  rating: number;
  modified: number;
}

interface MediasRatingMessage {
  library: string;
  rating: MediaRating;
}

// Watched events (user-specific)
// IMPORTANT: The `id` field uses external IDs, NOT local database IDs.
// Format: "provider:id" (e.g., "imdb:tt1234567", "trakt:123456", "tmdb:550")
// For movies: Uses the best available external ID (priority: imdb > trakt > tmdb > tvdb > slug)
// For episodes: Uses external IDs or falls back to local "redseat:{id}" if no external IDs exist
interface Watched {
  type: string;  // MediaType: "movie", "episode", etc.
  id: string;    // External ID in format "provider:value" (e.g., "imdb:tt1234567")
  userRef?: string;
  date: number;  // Timestamp when content was watched
  modified: number;
}

// Unwatched events (user-specific)
// NOTE: Different structure from Watched - contains ALL possible IDs for client matching
interface Unwatched {
  type: string;     // MediaType: "movie", "episode", etc.
  ids: string[];    // All possible IDs in format "provider:value" (e.g., ["imdb:tt1234567", "trakt:12345", "tmdb:550"])
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
    const { mediaId, progress, status } = data.ConvertProgress;
    console.log(`Converting ${mediaId}: ${progress}% - ${status}`);
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
    // Unwatched events contain ALL possible IDs for the content
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
| `GET /libraries/:libraryId/series/searchstream` | `name` (required), `ids` (optional) | Stream series search results |
| `GET /libraries/:libraryId/movies/searchstream` | `name` (required), `ids` (optional) | Stream movie search results |
| `GET /libraries/:libraryId/books/searchstream` | `name` (required), `ids` (optional) | Stream book search results |

### How It Works

Each SSE event has event type `results`. The data is a JSON object with a single key: the provider name, and the value is an array of results from that provider.

Results arrive one provider at a time. For series and movies, Trakt results are sent first, followed by each plugin (e.g., Anilist). For books, only plugin results are sent (no Trakt).

### Event Format

Each `results` event contains one provider's results:

```json
{"trakt": [{"metadata": {"serie": { ... }}, "images": []}]}
```

Then a second event for the next provider:

```json
{"Anilist": [{"metadata": {"serie": { ... }}, "images": [{"url": "...", "kind": "poster"}]}]}
```

The `metadata` field is a tagged enum with one of: `serie`, `movie`, `book`, `episode`, `person`, `media`.

### TypeScript Example

```typescript
const params = new URLSearchParams({ name: 'one piece' });
const eventSource = new EventSource(
  `/libraries/${libraryId}/series/searchstream?${params}`
);

// Accumulate results by provider
const resultsByProvider: Record<string, SearchResult[]> = {};

eventSource.addEventListener('results', (event) => {
  const data = JSON.parse(event.data);
  // data is e.g. { "trakt": [...] } or { "Anilist": [...] }
  for (const [provider, results] of Object.entries(data)) {
    resultsByProvider[provider] = results;
  }
  // Update UI with new results
  renderResults(resultsByProvider);
});

eventSource.onerror = () => {
  // Stream finished or errored — close the connection
  eventSource.close();
};
```

### Non-Streaming Alternative

The same search is available as a regular JSON endpoint that returns all providers at once:

| Endpoint | Description |
|----------|-------------|
| `GET /libraries/:libraryId/series/search` | Returns all results grouped by provider |
| `GET /libraries/:libraryId/movies/search` | Returns all results grouped by provider |
| `GET /libraries/:libraryId/books/search` | Returns all results grouped by provider |

Response format:

```json
{
  "trakt": [{"metadata": {"movie": { ... }}, "images": []}],
  "Anilist": [{"metadata": {"movie": { ... }}, "images": [...]}]
}
```

## Keepalive

The server sends a keepalive ping every 30 seconds to prevent connection timeouts. The ping is sent as a comment (`:ping`) which is ignored by the EventSource API.

## Error Handling

When a client falls behind and misses events (lag), the server will skip the missed events and continue with new ones. Consider implementing periodic full-sync if you need guaranteed delivery of all events.

## Watched/Unwatched Events

### Understanding the ID Format

The `watched` and `unwatched` events use **external IDs** (from providers like IMDb, Trakt, TMDb) rather than local database IDs. This allows watch history to be portable across different servers and sync with external services.

**ID Format**: `provider:value`

| Provider | Example | Content Types |
|----------|---------|---------------|
| `imdb` | `imdb:tt1234567` | Movies, Episodes |
| `trakt` | `trakt:123456` | Movies, Episodes, Series |
| `tmdb` | `tmdb:550` | Movies, Episodes, Series |
| `tvdb` | `tvdb:78901` | Episodes, Series |
| `slug` | `slug:the-matrix` | Movies, Series |
| `redseat` | `redseat:abc123` | Local fallback (episodes only) |

**ID Selection Priority**:
- **Movies**: Uses the best external ID (priority: imdb > trakt > tmdb > slug)
- **Episodes**: Uses external IDs, or falls back to local `redseat:` ID if no external IDs exist

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

**Direct History** (requires knowing the external ID): `POST /users/me/history`
```json
{
  "type": "movie",
  "id": "imdb:tt1234567",
  "date": 1705766400000
}
```

#### Unmark as Watched (Remove from History)

**Movies**: `DELETE /libraries/:libraryId/movies/:id/watched`

**Episodes**: `DELETE /libraries/:libraryId/series/:serieId/seasons/:season/episodes/:number/watched`

**Direct History** (with multiple possible IDs): `DELETE /users/me/history`
```json
{
  "type": "movie",
  "ids": ["imdb:tt1234567", "trakt:12345", "tmdb:550"]
}
```

The delete endpoints accept multiple IDs because the watched entry could have been created with any of the available external IDs. The server will try to delete entries matching any of the provided IDs.

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

Since SSE events use external IDs, you need to match them against your local content's external IDs:

```typescript
interface LocalMovie {
  id: string;        // Local database ID
  imdb?: string;     // "tt1234567"
  trakt?: number;    // 12345
  tmdb?: number;     // 550
}

// For Watched events (single ID)
function isMatchingWatchedEvent(movie: LocalMovie, eventId: string): boolean {
  const [provider, value] = eventId.split(':');
  switch (provider) {
    case 'imdb': return movie.imdb === value;
    case 'trakt': return movie.trakt?.toString() === value;
    case 'tmdb': return movie.tmdb?.toString() === value;
    default: return false;
  }
}

// For Unwatched events (array of IDs)
function isMatchingUnwatchedEvent(movie: LocalMovie, eventIds: string[]): boolean {
  return eventIds.some(eventId => isMatchingWatchedEvent(movie, eventId));
}
```

## Offline Sync for Watch History

When clients are offline or disconnected from SSE, they can miss `unwatched` events. The REST API provides a mechanism to sync these missed deletions.

### How It Works

- **`date > 0`**: Item is actively watched (timestamp indicates when it was watched)
- **`date = 0`**: Item was unwatched/deleted (soft-deleted, kept for sync purposes)

When content is marked as unwatched, instead of being deleted from the database, the `date` field is set to `0` and the `modified` timestamp is updated. This allows clients to fetch all changes (including deletions) via the history API.

### Client Sync Flow

```typescript
// 1. Store last sync timestamp locally
let lastSyncTimestamp = localStorage.getItem('lastHistorySync') || '0';

// 2. Fetch all history changes since last sync, including deleted items
async function syncHistory() {
  const response = await fetch(
    `/users/me/history?after=${lastSyncTimestamp}&includeDeleted=true`
  );
  const items: Watched[] = await response.json();

  for (const item of items) {
    if (item.date > 0) {
      // Active watched item - add or update in local state
      addToLocalWatched(item);
    } else {
      // Deleted item (date = 0) - remove from local state
      removeFromLocalWatched(item.type, item.id);
    }

    // Track highest modified timestamp for next sync
    if (item.modified > parseInt(lastSyncTimestamp)) {
      lastSyncTimestamp = item.modified.toString();
    }
  }

  localStorage.setItem('lastHistorySync', lastSyncTimestamp);
}

// 3. Call on app startup and periodically while online
syncHistory();
```

### API Query Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `after` | number | Only return items modified after this timestamp (milliseconds) |
| `includeDeleted` | boolean | Include items with `date=0` (unwatched). Default: `false` |
| `types` | string[] | Filter by content types (e.g., `movie`, `episode`) |

### Example Response with Deleted Items

```json
[
  {
    "type": "movie",
    "id": "imdb:tt1234567",
    "userRef": "user123",
    "date": 1705766400000,
    "modified": 1705852800000
  },
  {
    "type": "movie",
    "id": "trakt:98765",
    "userRef": "user123",
    "date": 0,
    "modified": 1705939200000
  }
]
```

In this response:
- First item: Movie was watched at timestamp `1705766400000`
- Second item: Movie was unwatched (`date=0`), client should remove it from local state
