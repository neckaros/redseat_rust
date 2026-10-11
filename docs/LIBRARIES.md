# Library updates

`PATCH /libraries/:id` changes a library. It needs the Admin role on the library
and returns the library as it is after the change. Fields left out of the body
(`name`, `source`, `root`, `credentials`, `plugin`, `settings`) are not changed.

## Settings

`settings` is merged into the stored settings, as a JSON Merge Patch (RFC 7396).
Send only what changes:

| In the body | Effect |
|-------------|--------|
| setting left out | kept as it is |
| setting with a value | replaced by that value |
| setting set to `null` | cleared |

```
PATCH /libraries/{libraryId}
{ "settings": { "maxStreams": 3, "epgUrl": null } }
```

sets `maxStreams`, clears `epgUrl` and keeps every other setting. A list such as
`mapProgress` is replaced as a whole, not merged item by item.

Leaving a setting out no longer clears it: send `null` for that. Sending back the
whole settings object still works, but it writes every setting it holds, so a
change made elsewhere in the meantime is overwritten. Two requests that change
different settings are both kept.

### Available settings

| Setting | Type | Notes |
|---------|------|-------|
| `faceThreshold` | number | Above 0 and at most 1 |
| `ignoreGroups` | boolean | |
| `preductionModel` | string | |
| `embeddings` | boolean | See `EMBEDDINGS.md` |
| `embeddingsPaused` | boolean | Read-only, see below |
| `mapProgress` | `{ from, to }[]` | |
| `dataPath` | string | |
| `epgUrl` | string | IPTV |
| `maxStreams` | integer | IPTV, at least 1 |

`embeddingsPaused` belongs to the server. It changes only through
`POST /libraries/:id/medias/embeddings/pause` and `.../resume`; a value sent in
`settings` is ignored, whatever it is.

### Errors

| Status | When |
|--------|------|
| `422` | A setting the server does not know (a typo such as `maxStream`), or a value of the wrong type. The body names the setting. |
| `400` | A value out of range, with the reason in `error.value.message`. Only the settings present in the request are checked. |

Nothing is changed when a request is refused.
