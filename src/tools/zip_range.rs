use std::io::Read;

use flate2::read::DeflateDecoder;
use rs_plugin_common_interfaces::request::{RsRequest, RsRequestStatus};

use crate::{
    error::{Error, RsResult},
    plugins::sources::RsRequestHeader,
    routes::mw_range::RangeDefinition,
};

async fn fetch_range(request: &RsRequest, start: u64, end: u64) -> RsResult<Vec<u8>> {
    let range = Some(RangeDefinition {
        start: Some(start),
        end: Some(end),
    });
    let client = reqwest::Client::new();
    let response = client
        .get(&request.url)
        .add_request_headers(request, &range)?
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(Error::Error(format!(
            "ZIP range request failed with status: {}",
            response.status()
        )));
    }

    let bytes = response.bytes().await?;
    Ok(bytes.to_vec())
}

fn read_u16_le(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

fn read_u32_le(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

/// Extract a specific page from a ZIP archive accessible via HTTP range requests.
///
/// `page` is 1-based. `file_size` must be the exact byte length of the ZIP file.
///
/// Returns `(decompressed_bytes, filename)`.
pub async fn extract_zip_page_from_request(
    request: &RsRequest,
    page: usize,
    file_size: u64,
) -> RsResult<(Vec<u8>, Option<String>)> {
    if page == 0 {
        return Err(Error::Error("Page index must be >= 1".to_string()));
    }

    // ── Step 1: fetch the tail chunk (up to 65 536 bytes) to locate the EOCD ──
    let chunk_size: u64 = 65536;
    let fetch_start = file_size.saturating_sub(chunk_size);
    let tail = fetch_range(request, fetch_start, file_size - 1).await?;

    // ── Step 2: scan backwards for EOCD signature PK\x05\x06 ──
    const EOCD_SIG: [u8; 4] = [0x50, 0x4B, 0x05, 0x06];
    let eocd_rel = tail
        .windows(4)
        .rposition(|w| w == EOCD_SIG)
        .ok_or_else(|| Error::Error("ZIP EOCD signature not found in tail".to_string()))?;

    let eocd = &tail[eocd_rel..];
    if eocd.len() < 22 {
        return Err(Error::Error("EOCD record too short".to_string()));
    }

    let cd_size = read_u32_le(eocd, 12) as u64;
    let cd_offset = read_u32_le(eocd, 16) as u64;

    // ── Step 3: get central directory bytes (may already be in the tail) ──
    let cd: Vec<u8> =
        if cd_offset >= fetch_start && (cd_offset - fetch_start + cd_size) as usize <= tail.len()
        {
            let rel_start = (cd_offset - fetch_start) as usize;
            tail[rel_start..rel_start + cd_size as usize].to_vec()
        } else {
            fetch_range(request, cd_offset, cd_offset + cd_size - 1).await?
        };

    // ── Step 4: walk CD entries to find the entry at index (page - 1) ──
    const CD_SIG: [u8; 4] = [0x50, 0x4B, 0x01, 0x02];
    let target_index = page - 1;

    let mut pos = 0usize;
    let mut entry_count = 0usize;
    // (local_header_offset, compressed_size, compression_method, filename)
    let mut found: Option<(u64, u64, u16, String)> = None;

    while pos + 46 <= cd.len() {
        if cd[pos..pos + 4] != CD_SIG {
            break;
        }

        let compression = read_u16_le(&cd, pos + 10);
        let compressed_size = read_u32_le(&cd, pos + 20) as u64;
        let filename_len = read_u16_le(&cd, pos + 28) as usize;
        let extra_len = read_u16_le(&cd, pos + 30) as usize;
        let comment_len = read_u16_le(&cd, pos + 32) as usize;
        let local_offset = read_u32_le(&cd, pos + 42) as u64;

        if entry_count == target_index {
            let filename_end = pos + 46 + filename_len;
            if filename_end > cd.len() {
                return Err(Error::Error("CD entry filename extends beyond data".to_string()));
            }
            let filename = String::from_utf8_lossy(&cd[pos + 46..filename_end]).to_string();
            found = Some((local_offset, compressed_size, compression, filename));
            break;
        }

        pos += 46 + filename_len + extra_len + comment_len;
        entry_count += 1;
    }

    let (local_offset, compressed_size, compression, filename) = found.ok_or_else(|| {
        Error::Error(format!(
            "ZIP page {} not found (archive has {} entries)",
            page, entry_count
        ))
    })?;

    // ── Step 5: read local file header to determine data offset ──
    let lh = fetch_range(request, local_offset, local_offset + 29).await?;
    if lh.len() < 30 {
        return Err(Error::Error("Local file header response too short".to_string()));
    }
    let lh_filename_len = read_u16_le(&lh, 26) as u64;
    let lh_extra_len = read_u16_le(&lh, 28) as u64;
    let data_start = local_offset + 30 + lh_filename_len + lh_extra_len;

    // ── Step 6: fetch the compressed page data ──
    let compressed: Vec<u8> = if compressed_size > 0 {
        fetch_range(request, data_start, data_start + compressed_size - 1).await?
    } else {
        vec![]
    };

    // ── Step 7: decompress ──
    let data = match compression {
        0 => compressed, // Stored — no compression
        8 => {
            // Deflate (raw, no zlib header)
            let mut decoder = DeflateDecoder::new(compressed.as_slice());
            let mut out = Vec::new();
            decoder
                .read_to_end(&mut out)
                .map_err(|e| Error::Error(format!("Deflate decompression failed: {e}")))?;
            out
        }
        other => {
            return Err(Error::Error(format!(
                "Unsupported ZIP compression method: {other}"
            )));
        }
    };

    let name = if filename.is_empty() { None } else { Some(filename) };
    Ok((data, name))
}
