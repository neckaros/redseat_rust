# Redseat WASM Plugin Guide

This document explains how to scaffold, build, test, and install a new Redseat plugin.

It is based on:
- Redseat host code in `src/plugins/*` and `src/model/plugins/*`
- Example plugins:
  - `/Users/arnaudjezequel/Documents/dev/plugins/plugin-torbox`
  - `/Users/arnaudjezequel/Documents/dev/plugins/rs-plugin-anilist`
  - `/Users/arnaudjezequel/Developer/plugins/rs-plugin-lookup-jackett/src/lib.rs`

## 1. How plugins are loaded in Redseat

- Redseat loads `.wasm` files from the server local `plugins/` folder.
- On load, Redseat calls exported function `infos` to read `PluginInformation`.
- Capability-specific functions are called only when your plugin advertises the matching `PluginType`.
- A function that is intentionally unsupported should return HTTP-like code `404` (via Extism `WithReturnCode`). Redseat treats `404` as "not implemented/not applicable" and tries next plugin.

Important runtime detail:
- Uploading a `.wasm` with `/plugins/upload` does not reload automatically. Call `/plugins/reload` after upload.
- Uploading from GitHub repo with `/plugins/upload/repo` does reload automatically.

## 2. Scaffold a new plugin

### 2.1 Create project

```bash
cargo new --lib rs-plugin-myplugin
cd rs-plugin-myplugin
```

### 2.2 Configure `Cargo.toml`

```toml
[package]
name = "rs-plugin-myplugin"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
extism-pdk = "1.4.1"
rs-plugin-common-interfaces = "0.27.3"
serde = { version = "1", features = ["derive"] }
serde_json = "1"

[dev-dependencies]
extism = "1"
```

### 2.3 Minimal `src/lib.rs` (Request plugin example)

```rust
use extism_pdk::{plugin_fn, FnResult, Json, WithReturnCode};
use rs_plugin_common_interfaces::{
    request::{RsRequestPluginRequest, RsRequestStatus},
    PluginInformation, PluginType,
};

#[plugin_fn]
pub fn infos() -> FnResult<Json<PluginInformation>> {
    Ok(Json(PluginInformation {
        name: "myplugin".into(),
        capabilities: vec![PluginType::Request],
        version: 1,
        interface_version: 1,
        publisher: "yourname".into(),
        description: "My first Redseat plugin".into(),
        repo: Some("https://github.com/your-org/rs-plugin-myplugin".into()),
        credential_kind: None,
        settings: vec![],
        ..Default::default()
    }))
}

#[plugin_fn]
pub fn process(Json(req): Json<RsRequestPluginRequest>) -> FnResult<Json<rs_plugin_common_interfaces::RsRequest>> {
    let mut request = req.request;

    if !request.url.starts_with("myproto://") {
        return Err(WithReturnCode::new(extism_pdk::Error::msg("Not supported"), 404));
    }

    request.url = request.url.replacen("myproto://", "https://", 1);
    request.status = RsRequestStatus::FinalPublic;
    Ok(Json(request))
}
```

## 3. Capability -> exported functions

Use exact function names below (host calls are string-based):

| Capability | Required/Used exports | Input | Output |
|---|---|---|---|
| Always | `infos` | `""` | `PluginInformation` |
| `UrlParser` | `parse`, `expand` | `&str`, `RsLink` | `RsLink`, `String` |
| `Request` | `process`, `request_permanent` | `RsRequestPluginRequest` | `RsRequest` |
| `Request` (optional advanced) | `check_instant`, `request_add`, `get_progress`, `pause`, `remove` | `RsRequestPluginRequest` / `RsProcessingActionRequest` | `bool` / `RsRequestAddResponse` / `RsProcessingProgress` / `()` |
| `Lookup` | `lookup` | `RsLookupWrapper` | `RsLookupSourceResult` |
| `LookupMetadata` | `lookup_metadata`, `lookup_metadata_images` | `RsLookupWrapper` | `Vec<RsLookupMetadataResultWithImages>`, `Vec<ExternalImage>` |
| `Provider` | `download_request`, `upload_request`, `upload_response`, `remove_file`, `file_info` | `RsPluginRequest<...>` | `RsRequest`/`RsProviderAddResponse`/`RsProviderEntry`/`()` |
| `VideoConvert` | `get_convert_capabilities`, `convert`, `convert_status`, `convert_cancel`, `convert_link`, `convert_clean` | Video plugin request structs | video capability/status structs |
| OAuth creds | `exchange_token` | `RsPluginRequest<HashMap<String,String>>` | `PluginCredential` |

Note: Redseat currently calls `renew_crendentials` (typo in host name) if you implement credential refresh.

## 4. Build

```bash
rustup target add wasm32-unknown-unknown
cargo build --target wasm32-unknown-unknown --release
```

Artifact path:
```bash
target/wasm32-unknown-unknown/release/rs_plugin_myplugin.wasm
```

## 5. Quick local smoke tests (Extism CLI)

```bash
extism call ./target/wasm32-unknown-unknown/release/rs_plugin_myplugin.wasm infos
```

For JSON stdin calls (same style as `plugin-torbox`):
```bash
cat ./process_input.json | extism call ./target/wasm32-unknown-unknown/release/rs_plugin_myplugin.wasm --allow-host '*' --wasi process --stdin
```

## 6. Integration tests (AniList pattern)

`rs-plugin-anilist` uses host-level integration tests with `extism` crate and a real WASM artifact.

### 6.1 Add `tests/lookup_test.rs`

```rust
use extism::*;
use rs_plugin_common_interfaces::{
    lookup::{RsLookupQuery, RsLookupSerie, RsLookupWrapper},
};

fn build_plugin() -> Plugin {
    let wasm = Wasm::file("target/wasm32-unknown-unknown/release/rs_plugin_myplugin.wasm");
    let manifest = Manifest::new([wasm]).with_allowed_host("graphql.anilist.co");
    Plugin::new(&manifest, [], true).expect("Failed to create plugin")
}

#[test]
fn test_lookup_basic() {
    let mut plugin = build_plugin();

    let input = RsLookupWrapper {
        query: RsLookupQuery::Serie(RsLookupSerie {
            name: Some("One piece".to_string()),
            ids: None,
        }),
        credential: None,
        params: None,
    };

    let input_str = serde_json::to_string(&input).unwrap();
    let output = plugin
        .call::<&str, &[u8]>("lookup_metadata", &input_str)
        .expect("lookup_metadata call failed");

    let value: serde_json::Value = serde_json::from_slice(output).unwrap();
    assert!(value.as_array().map(|a| !a.is_empty()).unwrap_or(false));
}
```

### 6.2 Run tests

This is exactly how `rs-plugin-anilist` is run:

```bash
cargo build --target wasm32-unknown-unknown --release
cargo test --test lookup_test -- --nocapture
```

Recommendation:
- Keep pure data transforms in unit tests (`#[cfg(test)]` in `src/lib.rs`).
- Keep live API / end-to-end behavior in integration tests under `tests/`.

## 7. Install in Redseat

### 7.1 Upload local wasm

```bash
curl -X POST "$REDSEAT_URL/plugins/upload" \
  -H "Authorization: Bearer $TOKEN" \
  -F "file=@target/wasm32-unknown-unknown/release/rs_plugin_myplugin.wasm"

curl "$REDSEAT_URL/plugins/reload" \
  -H "Authorization: Bearer $TOKEN"
```

### 7.2 Install plugin in DB

- List loaded wasm plugins: `GET /plugins`
- Install one: `POST /plugins/install`

Example payload:
```json
{
  "path": "plugin_xxxxx.wasm",
  "type": "lookupMetadata"
}
```

`path` is the loaded wasm filename shown by `GET /plugins`.

## 8. Repo-based install/update flow

For `/plugins/upload/repo` and `/plugins/:id/reporefresh` to work:

- `infos.repo` should point to a GitHub repository URL.
- That repository must have a **latest release** with at least one `.wasm` asset.
- Redseat downloads the first `.wasm` asset from latest release.

The example plugins all use a GitHub Action that:
- builds `wasm32-unknown-unknown --release`
- publishes the `.wasm` file in a GitHub Release when `Cargo.toml` version changes.

## 9. Important types reference

Use these types from `rs-plugin-common-interfaces` instead of redefining your own payloads.

### Core plugin metadata/auth

| Type | Purpose |
|---|---|
| `PluginInformation` | Returned by `infos`; declares `name`, `capabilities`, `credential_kind`, `settings`, etc. |
| `PluginType` | Capability enum (`Request`, `Lookup`, `LookupMetadata`, `Provider`, `VideoConvert`, ...). |
| `CredentialType` | Credential contract (`Token`, `Password`, `Url`, `Oauth { url }`). |
| `PluginCredential` | Runtime credential passed to plugin calls. |
| `CustomParam` / `CustomParamTypes` | UI-exposed plugin settings schema shown in Redseat plugin params. |
| `RsPluginRequest<T>` | Generic wrapper used by some calls (`request`, `plugin_settings`, `credential`). |

### Request processing types

| Type | Purpose |
|---|---|
| `RsRequest` | Main media request object (`url`, `status`, `mime`, `filename`, `headers`, etc.). |
| `RsRequestStatus` | Request lifecycle (`Unprocessed`, `Intermediate`, `NeedFileSelection`, `FinalPublic`, ...). |
| `RsRequestPluginRequest` | Input for `process`, `request_permanent`, `check_instant`, `request_add`. |
| `RsRequestAddResponse` | Response from `request_add` for async processing services. |
| `RsProcessingActionRequest` | Input for `get_progress`, `pause`, `remove`. |
| `RsProcessingProgress` / `RsProcessingStatus` | Progress/status payload for async request processing. |

### Lookup and metadata types

| Type | Purpose |
|---|---|
| `RsLookupQuery` | Lookup input enum (`Episode`, `Movie`, `Serie`, `Book`, ...). |
| `RsLookupWrapper` | Input wrapper for lookup calls (`query`, `credential`, `params`). |
| `RsLookupSourceResult` | Output for `lookup` (`Requests`, `NotFound`, `NotApplicable`). |
| `RsLookupMetadataResultWithImages` | Output for `lookup_metadata` (metadata + image list + optional lookup tags/people). |
| `ExternalImage` | Output item for `lookup_metadata_images`. |

### URL parser types

| Type | Purpose |
|---|---|
| `RsLink` / `RsLinkType` | Input/output of `parse` and `expand` URL parser functions. |

### Provider types

| Type | Purpose |
|---|---|
| `RsProviderPath` | Identifies file in provider storage (`root`, `source`). |
| `RsProviderAddRequest` | Input for upload initialization. |
| `RsProviderAddResponse` | Upload request/target response from provider plugin. |
| `RsProviderEntry` / `RsProviderEntryType` | File metadata (`size`, `mimetype`, timestamps, kind). |

### Video conversion types

| Type | Purpose |
|---|---|
| `RsVideoCapabilities` | Capabilities returned by `get_convert_capabilities`. |
| `RsVideoTranscodeJobPluginRequest` | Input for `convert`. |
| `RsVideoTranscodeJobPluginAction` | Input for `convert_status`, `convert_cancel`, `convert_link`, `convert_clean`. |
| `RsVideoTranscodeJobStatus` / `RsVideoTranscodeStatus` | Conversion job status/progress. |
| `RsVideoTranscodeCancelResponse` | Response for cancel action. |
| `VideoConvertRequest` | Conversion options (format, codec, overlays, intervals, etc.). |

### Example import block

```rust
use rs_plugin_common_interfaces::{
    lookup::{RsLookupQuery, RsLookupSourceResult, RsLookupWrapper, RsLookupMetadataResultWithImages},
    provider::{RsProviderAddRequest, RsProviderAddResponse, RsProviderEntry, RsProviderPath},
    request::{RsProcessingActionRequest, RsProcessingProgress, RsRequest, RsRequestAddResponse, RsRequestPluginRequest, RsRequestStatus},
    video::{RsVideoCapabilities, RsVideoTranscodeCancelResponse, RsVideoTranscodeJobPluginAction, RsVideoTranscodeJobPluginRequest, RsVideoTranscodeJobStatus, VideoConvertRequest},
    CredentialType, CustomParam, CustomParamTypes, ExternalImage, PluginCredential, PluginInformation, PluginType, RsLink, RsPluginRequest,
};
```

## 10. Common pitfalls

- Function name mismatch: host calls exact names (for example `lookup_metadata_images`).
- Wrong serialization casing: use interface structs from `rs-plugin-common-interfaces` and keep camelCase JSON.
- Missing reload after local upload: call `/plugins/reload`.
- Returning generic errors for unsupported inputs: return `404` instead so host can try other plugins.
- Missing `crate-type = ["cdylib"]`: no usable wasm artifact.
