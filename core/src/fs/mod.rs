pub mod storage;
pub mod partition;

use memmap2::Mmap;
use std::fs;
use std::io;
use std::path::Path;
use std::ops::Deref;

use crate::fs::storage::EmulatedStorage;
use crate::fs::partition::{PartitionTable, PartitionId};

/// Memory-mapped host file (unchanged from original).
pub struct File {
    map: Mmap,
}

impl File {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, io::Error> {
        let file = fs::File::open(path)?;
        let map = unsafe { Mmap::map(&file)? };
        Ok(Self { map })
    }
}

impl Deref for File {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.map
    }
}

/// Top-level emulated storage state representing the Switch's internal
/// 32 GB UFS/eMMC drive and its partition layout.
pub struct Nand {
    pub storage: EmulatedStorage,
    pub partitions: PartitionTable,
}

impl Nand {
    /// Open (or create) a NAND image at the given path with the default
    /// Switch partition layout.
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let storage = EmulatedStorage::open(path)?;
        let partitions = PartitionTable::new_default(storage.total_blocks());
        Ok(Self { storage, partitions })
    }

    /// Read bytes from a named partition at the given offset.
    pub fn read(&mut self, id: PartitionId, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        self.partitions.read_partition(&mut self.storage, id, offset, buf)
    }

    /// Write bytes to a named partition at the given offset.
    pub fn write(&mut self, id: PartitionId, offset: u64, buf: &[u8]) -> io::Result<()> {
        self.partitions.write_partition(&mut self.storage, id, offset, buf)
    }

    /// Flush pending writes to disk.
    pub fn flush(&mut self) -> io::Result<()> {
        self.storage.flush()
    }
}