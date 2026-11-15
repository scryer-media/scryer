//! Evidence kept for one unit of work, so no path is probed twice in it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::cache::key_of;
use super::{Disk, DiskError, Listed, PathReading};

/// One scan, import or move's view of the disk. It reads through the owner
/// and remembers every answer, including what its own listings said, until it
/// is dropped. A deletion guard never decides from a pass: it reads fresh.
#[derive(Debug)]
pub struct DiskPass {
    disk: Disk,
    readings: HashMap<PathBuf, PathReading>,
    listings: HashMap<PathBuf, Vec<Listed>>,
}

impl DiskPass {
    pub(crate) fn new(disk: Disk) -> Self {
        Self {
            disk,
            readings: HashMap::new(),
            listings: HashMap::new(),
        }
    }

    /// What is at `path`: from this pass's evidence when it has any, else one
    /// fresh read. Blocks on the disk.
    pub fn reading(&mut self, path: &Path) -> PathReading {
        let key = key_of(path);
        if let Some(reading) = self.readings.get(&key) {
            return *reading;
        }
        let reading = self.disk.fresh_blocking(path);
        self.readings.insert(key, reading);
        reading
    }

    /// The entries of `dir`, listed once per pass. Each entry's reading
    /// becomes evidence for [`Self::reading`].
    pub fn list(&mut self, dir: &Path) -> Result<&[Listed], DiskError> {
        let key = key_of(dir);
        if !self.listings.contains_key(&key) {
            let listed = self.disk.list_blocking(dir)?;
            for entry in &listed {
                if entry.reading != PathReading::NotObserved {
                    self.readings.insert(key_of(&entry.path), entry.reading);
                }
            }
            self.listings.insert(key.clone(), listed);
        }
        Ok(self
            .listings
            .get(&key)
            .map(Vec::as_slice)
            .unwrap_or_default())
    }

    /// Drop the evidence for `path` and everything under it, after the pass
    /// changed it.
    pub fn forget(&mut self, path: &Path) {
        let key = key_of(path);
        let parent = key.parent().map(Path::to_path_buf);
        let stale = |entry: &PathBuf| entry.starts_with(&key) || Some(entry) == parent.as_ref();
        self.readings.retain(|entry, _| !stale(entry));
        self.listings.retain(|entry, _| !stale(entry));
    }
}
