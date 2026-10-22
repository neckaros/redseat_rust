# Library stats

`GET /libraries/:id/stats` describes a library: its medias, the space of its
storage, its other entities, the work still to do on it, its last backup and what
Redseat stores for it. It needs the Admin role on the library; a share token is
refused, whatever it shares.

## Where the code is

| Part | Location |
| --- | --- |
| Counts from the library database | `src/model/store/sql/library/stats.rs` |
| Assembling the response, access rule | `src/model/library_stats.rs` |
| Storage of a provider | `storage` of the `Source` trait, `src/plugins/sources/` |
| Plugin call `storage_info` | `provider_storage` in `src/plugins/provider.rs` |
| Route | `src/routes/libraries.rs` |

## Response

```json
{
  "medias": {
    "count": 69503,
    "size": 312579635071,
    "byType": {
      "photo": { "count": 60809, "size": 128744931155, "duration": 185 },
      "video": { "count": 8694, "size": 183834703916, "duration": 199944 }
    },
    "firstCreated": 514911900000,
    "lastCreated": 1792531145000,
    "lastAdded": 1792534285600
  },
  "storage": { "used": 3880600000000, "free": 120200000000, "total": 4000800000000, "volume": "E:" },
  "entities": { "movies": 0, "series": 40, "episodes": 15, "people": 276, "tags": 3328, "books": 0, "channels": 0 },
  "duplicates": { "groups": 109, "extraMedias": 119, "extraSize": 608148606 },
  "processing": {
    "faces": { "pending": 17, "failed": 1008, "unassigned": 14913 },
    "embeddings": { "total": 69503, "embedded": 69000, "pending": 500, "failed": 3 },
    "encryption": { "phase": "idle", "processedItems": 0, "totalItems": 0, "passwordProtected": false }
  },
  "lastBackup": { "id": "backup-id", "name": "Nightly", "date": 1788641525855, "size": 448518320154 },
  "server": { "databaseSize": 210000000, "dataSize": 1100000000 }
}
```

Sizes are in bytes, durations in seconds, dates in milliseconds since the epoch.

| Field | Meaning |
| --- | --- |
| `medias` | Every media of the library. `byType` holds only the types present; a type's `duration` is left out when none of its medias has one. `firstCreated` and `lastCreated` are the oldest and newest creation dates, `lastAdded` the last time a media was added. |
| `storage` | The storage the library is on. See [Storage](#storage). |
| `entities` | Rows of each table. `episodes` counts the episodes known from metadata, with a file or not. |
| `duplicates` | Medias with the same content hash. `groups` is the number of contents stored more than once, `extraMedias` the medias beyond the first of each group, `extraSize` the bytes they take. |
| `processing.faces` | `pending`: medias the face scan has still to process. `failed`: medias it failed on, which it does not try again. `unassigned`: faces found but not attached to a person. |
| `processing.embeddings` | The counts of `GET /libraries/:id/medias/embeddings/status`, see `EMBEDDINGS.md`. Present whether embeddings are on or not. |
| `processing.encryption` | The body of `GET /libraries/:id/encryption`. |
| `lastBackup` | The backup of this library that completed last. `size` is what the backup holds at its destination, every kept version included. Left out until a backup of the library has completed. |
| `server` | What Redseat stores for the library on the server: `databaseSize` is the library database with its write-ahead log, `dataSize` the data folder (thumbnails, portraits, faces, caches): the `dataPath` setting, otherwise `.redseat` in the root of a path library and `libraries/<id>` in the server folder for the others. |

## Storage

`storage` comes from the provider and holds what it knows: every field is optional.
It is left out for a virtual library, when the provider cannot tell, and when it
does not answer within 10 seconds (the error is logged).

| Field | Meaning |
| --- | --- |
| `used` | Bytes used on the storage, by everything on it and not only this library (`medias.size` is the library). |
| `free` | Bytes that can still be written. |
| `total` | Size of the storage. A storage without a quota leaves out `free` and `total`. |
| `volume` | Libraries with the same value are on the same storage: count its space once when adding libraries up. |

A **path library** reports the disk holding its root, as `df` does: on a Linux disk
with reserved blocks, `used + free` is less than `total`. `volume` is the drive or
share on Windows (`E:`, `\\nas\media`), the device number elsewhere. The stats ask
the disk directly: they do not wait for an encryption change of the library.

A **plugin library** reports what its plugin returns from the optional export
`storage_info`. A plugin without that export reports nothing.

### Plugin export `storage_info`

Input: `RsPluginRequest<RsProviderPath>`, with the library root in `root` and an
empty `source`, plus the plugin settings and credential as for the other provider
exports.

Output, every field optional, sizes in bytes:

```json
{ "used": 512000000000, "free": 488000000000, "total": 1000000000000, "volume": "account-id" }
```

Leave out `free` and `total` when the account has no quota. Set `volume` to
something that identifies the account, so that two libraries on the same account
are counted once. An error from the plugin is logged and `storage` is left out.

## Cost

Everything but the storage and the data folder comes from the library database,
in about 100 ms for 70,000 medias. The data folder is walked at each request: 44 ms
for 90,000 files when the disk cache is warm, more than a second when it is not.
