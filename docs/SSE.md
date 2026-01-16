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
| `people` | People created/updated/deleted | Library read access |
| `tags` | Tags created/updated/deleted | Library read access |
| `backups` | Backup job events | Library admin or server admin |
| `backups-files` | Backup file progress | Server admin only |
| `media_progress` | Playback position tracking | User-specific (only progress owner) |
| `media_rating` | Media rating changes | User-specific (only rating owner) |

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
  media: Media;
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
  | { People: PeopleMessage }
  | { Tags: TagMessage }
  | { Backups: BackupMessage }
  | { BackupsFiles: BackupFileProgress }
  | { MediaProgress: MediasProgressMessage }
  | { MediaRating: MediasRatingMessage };
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
      'convert_progress', 'episodes', 'series', 'movies',
      'people', 'tags', 'backups', 'backups-files', 'media_progress',
      'media_rating'
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
      'convert_progress', 'episodes', 'series', 'movies',
      'people', 'tags', 'backups', 'backups-files', 'media_progress',
      'media_rating'
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

## Keepalive

The server sends a keepalive ping every 30 seconds to prevent connection timeouts. The ping is sent as a comment (`:ping`) which is ignored by the EventSource API.

## Error Handling

When a client falls behind and misses events (lag), the server will skip the missed events and continue with new ones. Consider implementing periodic full-sync if you need guaranteed delivery of all events.
