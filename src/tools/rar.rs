use std::path::{Path, PathBuf};

use unrar_ng::Archive;

use crate::error::{RsError, RsResult};

/// Album pages are expected to be images. This also limits a malformed archive from
/// making a single request allocate an unreasonable amount of memory.
pub const MAX_RAR_PAGE_SIZE: u64 = 512 * 1024 * 1024;
const MAX_RAR_SOLID_PREFIX_SIZE: u64 = 512 * 1024 * 1024;
const MAX_RAR_ENTRIES: usize = 65_536;

#[derive(Debug)]
struct RarMember {
    archive_index: usize,
    name: PathBuf,
    unpacked_size: u64,
}

#[derive(Debug)]
pub struct RarPage {
    pub data: Vec<u8>,
    pub name: String,
}

fn rar_error(context: &str, error: impl std::fmt::Display) -> RsError {
    RsError::Error(format!("{context}: {error}"))
}

fn add_to_solid_prefix_size(total: &mut u64, entry_size: u64) -> RsResult<()> {
    *total = total.checked_add(entry_size).ok_or_else(|| {
        RsError::Error("RAR solid prefix uncompressed size overflowed".to_string())
    })?;
    if *total > MAX_RAR_SOLID_PREFIX_SIZE {
        return Err(RsError::Error(format!(
            "RAR solid prefix is too large ({total} bytes; limit is {MAX_RAR_SOLID_PREFIX_SIZE} bytes)"
        )));
    }
    Ok(())
}

pub fn is_image_name(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "avif"
                    | "bmp"
                    | "gif"
                    | "heic"
                    | "heif"
                    | "j2k"
                    | "jp2"
                    | "jpeg"
                    | "jpg"
                    | "jxl"
                    | "png"
                    | "tif"
                    | "tiff"
                    | "webp"
            )
        })
        .unwrap_or(false)
}

fn list_members(path: &Path) -> RsResult<Vec<RarMember>> {
    let archive = Archive::new(path)
        .open_for_listing()
        .map_err(|error| rar_error("Unable to open RAR archive", error))?;

    let mut files = Vec::new();
    for (archive_index, entry) in archive.enumerate() {
        if archive_index >= MAX_RAR_ENTRIES {
            return Err(RsError::Error(format!(
                "RAR contains more than {MAX_RAR_ENTRIES} entries"
            )));
        }
        let entry = entry.map_err(|error| rar_error("Unable to list RAR archive", error))?;
        if entry.is_file() {
            files.push(RarMember {
                archive_index,
                name: entry.filename,
                unpacked_size: entry.unpacked_size,
            });
        }
    }

    let contains_images = files.iter().any(|entry| is_image_name(&entry.name));
    if contains_images {
        files.retain(|entry| is_image_name(&entry.name));
    }
    Ok(files)
}

pub fn count_rar_pages(path: &Path) -> RsResult<usize> {
    Ok(list_members(path)?.len())
}

/// Read one 1-based page from a RAR/CBR archive.
///
/// UnRAR archives are sequential (especially solid archives), so selecting a page requires
/// walking preceding headers. Run this function with `spawn_blocking` from async code.
pub fn read_rar_page(path: &Path, page: usize) -> RsResult<RarPage> {
    let page_index = page
        .checked_sub(1)
        .ok_or_else(|| RsError::Error("Page index must be >= 1".to_string()))?;
    let members = list_members(path)?;
    let member = members.get(page_index).ok_or_else(|| {
        RsError::Error(format!(
            "Unable to get RAR page {page}. Files in archive: {}",
            members.len()
        ))
    })?;

    if member.unpacked_size > MAX_RAR_PAGE_SIZE {
        return Err(RsError::Error(format!(
            "RAR page is too large ({} bytes; limit is {} bytes)",
            member.unpacked_size, MAX_RAR_PAGE_SIZE
        )));
    }

    let target_index = member.archive_index;
    let expected_size = member.unpacked_size;
    let name = member.name.to_string_lossy().into_owned();
    let mut archive = Archive::new(path)
        .open_for_processing()
        .map_err(|error| rar_error("Unable to open RAR archive", error))?;
    let is_solid = archive.is_solid();

    let mut archive_index = 0usize;
    let mut solid_prefix_size = 0u64;
    while let Some(entry) = archive
        .read_header()
        .map_err(|error| rar_error("Unable to read RAR header", error))?
    {
        if archive_index == target_index {
            let (data, _) = entry
                .read()
                .map_err(|error| rar_error("Unable to read RAR page", error))?;
            if data.len() as u64 != expected_size {
                return Err(RsError::Error(format!(
                    "RAR page size mismatch: expected {expected_size} bytes, read {} bytes",
                    data.len()
                )));
            }
            return Ok(RarPage { data, name });
        }
        archive = if is_solid {
            add_to_solid_prefix_size(&mut solid_prefix_size, entry.entry().unpacked_size)?;
            entry
                .test()
                .map_err(|error| rar_error("Unable to process preceding solid RAR entry", error))?
        } else {
            entry
                .skip()
                .map_err(|error| rar_error("Unable to skip RAR entry", error))?
        };
        archive_index += 1;
    }

    Err(RsError::Error(format!(
        "Unable to get RAR page {page}. Archive ended unexpectedly"
    )))
}

#[cfg(test)]
mod tests {
    use super::{add_to_solid_prefix_size, is_image_name, MAX_RAR_SOLID_PREFIX_SIZE};
    use std::path::Path;

    #[test]
    fn identifies_album_page_extensions_case_insensitively() {
        assert!(is_image_name(Path::new("pages/001.JPG")));
        assert!(is_image_name(Path::new("002.avif")));
        assert!(!is_image_name(Path::new("ComicInfo.xml")));
        assert!(!is_image_name(Path::new("pages/")));
    }

    #[test]
    fn bounds_cumulative_solid_prefix_decompression() {
        let mut total = MAX_RAR_SOLID_PREFIX_SIZE - 1;
        add_to_solid_prefix_size(&mut total, 1).unwrap();

        let error = add_to_solid_prefix_size(&mut total, 1).unwrap_err();
        assert!(error.to_string().contains("RAR solid prefix is too large"));
    }

    #[test]
    fn rejects_solid_prefix_size_overflow() {
        let mut total = u64::MAX;
        let error = add_to_solid_prefix_size(&mut total, 1).unwrap_err();
        assert!(error.to_string().contains("overflowed"));
    }
}
