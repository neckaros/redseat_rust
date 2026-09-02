use std::{io::Read, path::Path};

use flate2::read::DeflateDecoder;
use futures::TryStreamExt;
use rs_plugin_common_interfaces::request::RsRequest;

use crate::{
    error::{Error, RsResult},
    plugins::sources::RsRequestHeader,
    routes::mw_range::RangeDefinition,
    tools::rar::is_image_name,
};

const EOCD_MIN_SIZE: u64 = 22;
const EOCD_MAX_SIZE: u64 = EOCD_MIN_SIZE + u16::MAX as u64;
const MAX_CENTRAL_DIRECTORY_SIZE: u64 = 16 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: u64 = 65_536;
const MAX_PAGE_SIZE: u64 = 512 * 1024 * 1024;
const MAX_COMPRESSED_PAGE_SIZE: u64 = 512 * 1024 * 1024;

#[derive(Debug)]
struct CentralDirectoryEntry {
    local_header_offset: u64,
    compressed_size: u64,
    uncompressed_size: u64,
    crc32: u32,
    compression: u16,
    flags: u16,
    filename: String,
}

fn invalid_zip(message: impl Into<String>) -> Error {
    Error::Error(message.into())
}

async fn fetch_range(
    client: &reqwest::Client,
    request: &RsRequest,
    start: u64,
    end: u64,
) -> RsResult<Vec<u8>> {
    if start > end {
        return Err(invalid_zip(format!("Invalid ZIP byte range {start}-{end}")));
    }
    let range = Some(RangeDefinition {
        start: Some(start),
        end: Some(end),
    });
    let response = client
        .get(&request.url)
        .add_request_headers(request, &range)?
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .send()
        .await?;

    if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(invalid_zip(format!(
            "ZIP source did not honor byte range {start}-{end} (status {})",
            response.status()
        )));
    }

    let expected = end - start + 1;
    let expected_content_range = format!("bytes {start}-{end}/");
    let content_range = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok());
    if !content_range.is_some_and(|value| value.starts_with(&expected_content_range)) {
        return Err(invalid_zip(format!(
            "ZIP source returned an invalid Content-Range for {start}-{end}"
        )));
    }
    if response
        .content_length()
        .is_some_and(|content_length| content_length != expected)
    {
        return Err(invalid_zip(format!(
            "ZIP range {start}-{end} returned an unexpected Content-Length"
        )));
    }

    let mut bytes = Vec::with_capacity(expected.min(16 * 1024 * 1024) as usize);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.try_next().await? {
        if bytes.len() as u64 + chunk.len() as u64 > expected {
            return Err(invalid_zip(format!(
                "ZIP range {start}-{end} returned more than {expected} bytes"
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.len() as u64 != expected {
        return Err(invalid_zip(format!(
            "ZIP range {start}-{end} returned {} bytes instead of {expected}",
            bytes.len()
        )));
    }
    Ok(bytes)
}

fn read_u16_le(data: &[u8], offset: usize) -> RsResult<u16> {
    let bytes = data
        .get(offset..offset + 2)
        .ok_or_else(|| invalid_zip("Truncated ZIP structure"))?;
    Ok(u16::from_le_bytes(bytes.try_into().unwrap()))
}

fn read_u32_le(data: &[u8], offset: usize) -> RsResult<u32> {
    let bytes = data
        .get(offset..offset + 4)
        .ok_or_else(|| invalid_zip("Truncated ZIP structure"))?;
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
}

fn read_u64_le(data: &[u8], offset: usize) -> RsResult<u64> {
    let bytes = data
        .get(offset..offset + 8)
        .ok_or_else(|| invalid_zip("Truncated ZIP64 structure"))?;
    Ok(u64::from_le_bytes(bytes.try_into().unwrap()))
}

fn locate_eocd(tail: &[u8]) -> RsResult<usize> {
    const EOCD_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];
    for position in (0..tail.len().saturating_sub(3)).rev() {
        if tail.get(position..position + 4) != Some(EOCD_SIGNATURE.as_slice())
            || position + EOCD_MIN_SIZE as usize > tail.len()
        {
            continue;
        }
        let comment_length = read_u16_le(tail, position + 20)? as usize;
        if position + EOCD_MIN_SIZE as usize + comment_length == tail.len() {
            return Ok(position);
        }
    }
    Err(invalid_zip("ZIP end-of-central-directory record not found"))
}

fn parse_zip64_values(
    extra: &[u8],
    uncompressed_32: u32,
    compressed_32: u32,
    offset_32: u32,
) -> RsResult<(u64, u64, u64)> {
    let mut position = 0usize;
    while position + 4 <= extra.len() {
        let header_id = read_u16_le(extra, position)?;
        let size = read_u16_le(extra, position + 2)? as usize;
        let start = position + 4;
        let end = start
            .checked_add(size)
            .filter(|end| *end <= extra.len())
            .ok_or_else(|| invalid_zip("Invalid ZIP extra field size"))?;
        if header_id == 0x0001 {
            let field = &extra[start..end];
            let mut cursor = 0usize;
            let mut next = || {
                let value = read_u64_le(field, cursor)?;
                cursor += 8;
                Ok::<u64, Error>(value)
            };
            let uncompressed = if uncompressed_32 == u32::MAX {
                next()?
            } else {
                uncompressed_32 as u64
            };
            let compressed = if compressed_32 == u32::MAX {
                next()?
            } else {
                compressed_32 as u64
            };
            let offset = if offset_32 == u32::MAX {
                next()?
            } else {
                offset_32 as u64
            };
            return Ok((uncompressed, compressed, offset));
        }
        position = end;
    }

    if uncompressed_32 == u32::MAX || compressed_32 == u32::MAX || offset_32 == u32::MAX {
        Err(invalid_zip("ZIP64 entry is missing its ZIP64 extra field"))
    } else {
        Ok((
            uncompressed_32 as u64,
            compressed_32 as u64,
            offset_32 as u64,
        ))
    }
}

fn parse_central_directory(
    data: &[u8],
    declared_entries: u64,
) -> RsResult<Vec<CentralDirectoryEntry>> {
    const CDR_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x01, 0x02];
    let mut entries = Vec::with_capacity(declared_entries.min(MAX_ARCHIVE_ENTRIES) as usize);
    let mut position = 0usize;

    while position < data.len() {
        if data.get(position..position + 4) != Some(CDR_SIGNATURE.as_slice()) {
            return Err(invalid_zip("Invalid central-directory entry signature"));
        }
        let filename_length = read_u16_le(data, position + 28)? as usize;
        let extra_length = read_u16_le(data, position + 30)? as usize;
        let comment_length = read_u16_le(data, position + 32)? as usize;
        let filename_start = position + 46;
        let filename_end = filename_start
            .checked_add(filename_length)
            .filter(|end| *end <= data.len())
            .ok_or_else(|| invalid_zip("ZIP filename extends beyond central directory"))?;
        let extra_end = filename_end
            .checked_add(extra_length)
            .filter(|end| *end <= data.len())
            .ok_or_else(|| invalid_zip("ZIP extra field extends beyond central directory"))?;
        let next_position = extra_end
            .checked_add(comment_length)
            .filter(|end| *end <= data.len())
            .ok_or_else(|| invalid_zip("ZIP comment extends beyond central directory"))?;

        let uncompressed_32 = read_u32_le(data, position + 24)?;
        let compressed_32 = read_u32_le(data, position + 20)?;
        let offset_32 = read_u32_le(data, position + 42)?;
        let (uncompressed_size, compressed_size, local_header_offset) = parse_zip64_values(
            &data[filename_end..extra_end],
            uncompressed_32,
            compressed_32,
            offset_32,
        )?;
        entries.push(CentralDirectoryEntry {
            local_header_offset,
            compressed_size,
            uncompressed_size,
            crc32: read_u32_le(data, position + 16)?,
            compression: read_u16_le(data, position + 10)?,
            flags: read_u16_le(data, position + 8)?,
            filename: String::from_utf8_lossy(&data[filename_start..filename_end]).into_owned(),
        });
        if entries.len() as u64 > MAX_ARCHIVE_ENTRIES {
            return Err(invalid_zip("ZIP contains too many entries"));
        }
        position = next_position;
    }

    if entries.len() as u64 != declared_entries {
        return Err(invalid_zip(format!(
            "ZIP central directory declares {declared_entries} entries but contains {}",
            entries.len()
        )));
    }
    Ok(entries)
}

fn select_page(
    entries: Vec<CentralDirectoryEntry>,
    page: usize,
) -> RsResult<CentralDirectoryEntry> {
    let files = entries
        .into_iter()
        .filter(|entry| !entry.filename.ends_with('/'))
        .collect::<Vec<_>>();
    let contains_images = files
        .iter()
        .any(|entry| is_image_name(Path::new(&entry.filename)));
    let pages = files
        .into_iter()
        .filter(|entry| !contains_images || is_image_name(Path::new(&entry.filename)))
        .collect::<Vec<_>>();
    let index = page
        .checked_sub(1)
        .ok_or_else(|| invalid_zip("Page index must be >= 1"))?;
    let page_count = pages.len();
    pages.into_iter().nth(index).ok_or_else(|| {
        invalid_zip(format!(
            "Unable to get ZIP page {page}. Pages in archive: {page_count}"
        ))
    })
}

fn count_pages(entries: &[CentralDirectoryEntry]) -> usize {
    let contains_images = entries
        .iter()
        .any(|entry| !entry.filename.ends_with('/') && is_image_name(Path::new(&entry.filename)));
    entries
        .iter()
        .filter(|entry| {
            !entry.filename.ends_with('/')
                && (!contains_images || is_image_name(Path::new(&entry.filename)))
        })
        .count()
}

async fn read_central_directory(
    client: &reqwest::Client,
    request: &RsRequest,
    file_size: u64,
) -> RsResult<Vec<CentralDirectoryEntry>> {
    if file_size < EOCD_MIN_SIZE {
        return Err(invalid_zip("ZIP file is too short"));
    }

    let tail_start = file_size.saturating_sub(EOCD_MAX_SIZE);
    let tail = fetch_range(client, request, tail_start, file_size - 1).await?;
    let eocd_position = locate_eocd(&tail)?;
    let eocd = &tail[eocd_position..];
    if read_u16_le(eocd, 4)? != 0
        || read_u16_le(eocd, 6)? != 0
        || read_u16_le(eocd, 8)? != read_u16_le(eocd, 10)?
    {
        return Err(invalid_zip("Multi-disk ZIP archives are not supported"));
    }

    let entries_16 = read_u16_le(eocd, 10)?;
    let cd_size_32 = read_u32_le(eocd, 12)?;
    let cd_offset_32 = read_u32_le(eocd, 16)?;
    let needs_zip64 = entries_16 == u16::MAX || cd_size_32 == u32::MAX || cd_offset_32 == u32::MAX;
    let (declared_entries, cd_size, cd_offset) = if needs_zip64 {
        let eocd_absolute = tail_start + eocd_position as u64;
        if eocd_absolute < 20 {
            return Err(invalid_zip("ZIP64 locator is missing"));
        }
        let locator = fetch_range(client, request, eocd_absolute - 20, eocd_absolute - 1).await?;
        if read_u32_le(&locator, 0)? != 0x0706_4b50 {
            return Err(invalid_zip("Invalid ZIP64 locator signature"));
        }
        if read_u32_le(&locator, 4)? != 0 || read_u32_le(&locator, 16)? != 1 {
            return Err(invalid_zip("Multi-disk ZIP64 archives are not supported"));
        }
        let zip64_offset = read_u64_le(&locator, 8)?;
        let zip64_end = zip64_offset
            .checked_add(55)
            .filter(|end| *end < file_size)
            .ok_or_else(|| invalid_zip("ZIP64 end record is outside the file"))?;
        let zip64 = fetch_range(client, request, zip64_offset, zip64_end).await?;
        if read_u32_le(&zip64, 0)? != 0x0606_4b50 {
            return Err(invalid_zip("Invalid ZIP64 end record signature"));
        }
        if read_u32_le(&zip64, 16)? != 0
            || read_u32_le(&zip64, 20)? != 0
            || read_u64_le(&zip64, 24)? != read_u64_le(&zip64, 32)?
        {
            return Err(invalid_zip("Multi-disk ZIP64 archives are not supported"));
        }
        (
            read_u64_le(&zip64, 32)?,
            read_u64_le(&zip64, 40)?,
            read_u64_le(&zip64, 48)?,
        )
    } else {
        (entries_16 as u64, cd_size_32 as u64, cd_offset_32 as u64)
    };

    if declared_entries > MAX_ARCHIVE_ENTRIES {
        return Err(invalid_zip("ZIP contains too many entries"));
    }
    if cd_size > MAX_CENTRAL_DIRECTORY_SIZE {
        return Err(invalid_zip("ZIP central directory is too large"));
    }
    let cd_end = cd_offset
        .checked_add(cd_size)
        .filter(|end| *end <= file_size)
        .ok_or_else(|| invalid_zip("ZIP central directory is outside the file"))?;
    let central_directory = if cd_size == 0 {
        Vec::new()
    } else {
        fetch_range(client, request, cd_offset, cd_end - 1).await?
    };
    parse_central_directory(&central_directory, declared_entries)
}

/// Count album pages using only the ZIP central directory and HTTP byte ranges.
pub async fn count_zip_pages_from_request(request: &RsRequest, file_size: u64) -> RsResult<usize> {
    let client = reqwest::Client::new();
    let entries = read_central_directory(&client, request, file_size).await?;
    Ok(count_pages(&entries))
}

/// Extract a 1-based album page using HTTP ranges rather than downloading the full ZIP.
/// Supports ZIP32 and ZIP64 central directories and verifies page size and CRC.
pub async fn extract_zip_page_from_request(
    request: &RsRequest,
    page: usize,
    file_size: u64,
) -> RsResult<(Vec<u8>, Option<String>)> {
    let client = reqwest::Client::new();
    let entry = select_page(
        read_central_directory(&client, request, file_size).await?,
        page,
    )?;

    if entry.flags & 1 != 0 {
        return Err(invalid_zip("Encrypted ZIP entries are not supported"));
    }
    if entry.uncompressed_size > MAX_PAGE_SIZE {
        return Err(invalid_zip(format!(
            "ZIP page is too large ({} bytes; limit is {MAX_PAGE_SIZE} bytes)",
            entry.uncompressed_size
        )));
    }
    if entry.compressed_size > MAX_COMPRESSED_PAGE_SIZE {
        return Err(invalid_zip(format!(
            "Compressed ZIP page is too large ({} bytes; limit is {MAX_COMPRESSED_PAGE_SIZE} bytes)",
            entry.compressed_size
        )));
    }

    let local_header_end = entry
        .local_header_offset
        .checked_add(29)
        .filter(|end| *end < file_size)
        .ok_or_else(|| invalid_zip("ZIP local header is outside the file"))?;
    let local_header = fetch_range(
        &client,
        request,
        entry.local_header_offset,
        local_header_end,
    )
    .await?;
    if read_u32_le(&local_header, 0)? != 0x0403_4b50 {
        return Err(invalid_zip("Invalid ZIP local-header signature"));
    }
    if read_u16_le(&local_header, 8)? != entry.compression {
        return Err(invalid_zip(
            "ZIP compression method differs between headers",
        ));
    }
    if read_u16_le(&local_header, 6)? != entry.flags {
        return Err(invalid_zip("ZIP flags differ between headers"));
    }
    let filename_length = read_u16_le(&local_header, 26)? as u64;
    let extra_length = read_u16_le(&local_header, 28)? as u64;
    let data_start = entry
        .local_header_offset
        .checked_add(30)
        .and_then(|value| value.checked_add(filename_length))
        .and_then(|value| value.checked_add(extra_length))
        .ok_or_else(|| invalid_zip("ZIP page data offset overflow"))?;
    let data_end = data_start
        .checked_add(entry.compressed_size)
        .filter(|end| *end <= file_size)
        .ok_or_else(|| invalid_zip("ZIP page data is outside the file"))?;
    let compressed = if entry.compressed_size == 0 {
        Vec::new()
    } else {
        fetch_range(&client, request, data_start, data_end - 1).await?
    };

    let expected_size = entry.uncompressed_size;
    let expected_crc = entry.crc32;
    let compression = entry.compression;
    let data = tokio::task::spawn_blocking(move || -> RsResult<Vec<u8>> {
        let mut output = Vec::with_capacity(expected_size.min(16 * 1024 * 1024) as usize);
        match compression {
            0 => output = compressed,
            8 => {
                let decoder = DeflateDecoder::new(compressed.as_slice());
                decoder
                    .take(MAX_PAGE_SIZE + 1)
                    .read_to_end(&mut output)
                    .map_err(|error| {
                        invalid_zip(format!("Deflate decompression failed: {error}"))
                    })?;
            }
            other => {
                return Err(invalid_zip(format!(
                    "Unsupported ZIP compression method: {other}"
                )))
            }
        }
        if output.len() as u64 != expected_size {
            return Err(invalid_zip(format!(
                "ZIP page size mismatch: expected {expected_size} bytes, read {} bytes",
                output.len()
            )));
        }
        let actual_crc = crc32fast::hash(&output);
        if actual_crc != expected_crc {
            return Err(invalid_zip(format!(
                "ZIP page CRC mismatch: expected {expected_crc:08x}, got {actual_crc:08x}"
            )));
        }
        Ok(output)
    })
    .await??;

    Ok((data, Some(entry.filename)))
}

#[cfg(test)]
mod tests {
    use super::{locate_eocd, parse_zip64_values};

    #[test]
    fn finds_eocd_before_a_comment_that_contains_a_signature() {
        let mut bytes = vec![0x50, 0x4b, 0x05, 0x06];
        bytes.extend_from_slice(&[0; 16]);
        bytes.extend_from_slice(&4u16.to_le_bytes());
        bytes.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06]);
        assert_eq!(locate_eocd(&bytes).unwrap(), 0);
    }

    #[test]
    fn parses_zip64_entry_values_in_spec_order() {
        let mut extra = vec![0x01, 0x00, 24, 0];
        extra.extend_from_slice(&7u64.to_le_bytes());
        extra.extend_from_slice(&5u64.to_le_bytes());
        extra.extend_from_slice(&3u64.to_le_bytes());
        assert_eq!(
            parse_zip64_values(&extra, u32::MAX, u32::MAX, u32::MAX).unwrap(),
            (7, 5, 3)
        );
    }
}
