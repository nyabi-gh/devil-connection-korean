use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use dc_asar::{AsarArchive, EntryKind};

use crate::error::{InstallError, Result};
use crate::fsutil;
use crate::install::backup_path;

const PATCH_ZONES: &[&str] = &["data", "tyrano"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchState {
    Original,
    Patched,
}

pub fn stamp_path(asar_path: &Path) -> PathBuf {
    fsutil::with_suffix(asar_path, ".dcpatch")
}

pub fn patch_state(asar_path: &Path) -> Result<PatchState> {
    let current = AsarArchive::open(asar_path)?;
    let stamp = stamp_path(asar_path);

    match fs::read_to_string(&stamp) {
        Ok(text) if text.trim() == hex::encode(current.header_digest()) => Ok(PatchState::Patched),
        Ok(_) => Ok(PatchState::Original),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            unstamped_state(&current, &backup_path(asar_path))
        }
        Err(e) => Err(InstallError::io(&stamp, e)),
    }
}

pub(crate) fn write_stamp(asar_path: &Path, digest: [u8; 32]) -> Result<()> {
    let stamp = stamp_path(asar_path);
    fs::write(&stamp, hex::encode(digest)).map_err(|e| InstallError::io(&stamp, e))
}

fn unstamped_state(current: &AsarArchive, backup: &Path) -> Result<PatchState> {
    if !backup.is_file() {
        return Ok(PatchState::Original);
    }

    let backup = AsarArchive::open(backup)?;
    if backup.header_digest() == current.header_digest() {
        return Ok(PatchState::Original);
    }

    let current_files = fingerprints(current);
    let untouched = fingerprints(&backup)
        .into_iter()
        .filter(|(path, _)| !in_patch_zone(path))
        .all(|(path, expected)| {
            current_files
                .get(&path)
                .is_some_and(|actual| actual.matches(&expected))
        });

    Ok(if untouched {
        PatchState::Patched
    } else {
        PatchState::Original
    })
}

struct Fingerprint {
    size: u64,
    hash: Option<String>,
}

impl Fingerprint {
    fn matches(&self, other: &Fingerprint) -> bool {
        self.size == other.size
            && match (&self.hash, &other.hash) {
                (Some(a), Some(b)) => a == b,
                _ => true,
            }
    }
}

fn fingerprints(archive: &AsarArchive) -> BTreeMap<String, Fingerprint> {
    archive
        .entries()
        .into_iter()
        .filter_map(|entry| match entry.kind {
            EntryKind::File {
                size, integrity, ..
            } => Some((
                entry.path,
                Fingerprint {
                    size,
                    hash: integrity.map(|integrity| integrity.hash),
                },
            )),
            _ => None,
        })
        .collect()
}

fn in_patch_zone(path: &str) -> bool {
    PATCH_ZONES.iter().any(|zone| {
        path.strip_prefix(zone)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_zone_matches_whole_segments_only() {
        assert!(in_patch_zone("data/scenario/first.ks"));
        assert!(in_patch_zone("tyrano/tyrano.css"));
        assert!(!in_patch_zone("database.json"));
        assert!(!in_patch_zone("index.html"));
    }
}
