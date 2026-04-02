use std::io;

use crate::fs::storage::EmulatedStorage;

/// A GUID Partition Table entry for the emulated storage.
#[derive(Debug, Clone)]
pub struct GptEntry {
    /// Human-readable partition name (UTF-16 on real GPT, stored as String here).
    pub name: String,
    /// First LBA (logical block address) of the partition.
    pub start_block: u64,
    /// Last LBA (inclusive) of the partition.
    pub end_block: u64,
    /// 128-bit partition type GUID stored as raw bytes.
    pub type_guid: [u8; 16],
    /// 128-bit unique partition GUID stored as raw bytes.
    pub unique_guid: [u8; 16],
}

impl GptEntry {
    /// Size of this partition in blocks.
    pub fn block_count(&self) -> u64 {
        self.end_block - self.start_block + 1
    }

    /// Size of this partition in bytes (using 4 KiB blocks).
    pub fn size_bytes(&self, block_size: u64) -> u64 {
        self.block_count() * block_size
    }
}

/// Known partition identifiers for the emulated Switch storage layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartitionId {
    /// Factory calibration data (read-only in normal operation).
    ProdInfo,
    /// Factory calibration backup.
    ProdInfoF,
    /// Safe-mode firmware partition.
    Safe,
    /// Main operating system / firmware partition.
    System,
    /// User data (games, saves, etc.).
    User,
}

impl PartitionId {
    pub fn name(&self) -> &'static str {
        match self {
            Self::ProdInfo  => "PRODINFO",
            Self::ProdInfoF => "PRODINFOF",
            Self::Safe      => "SAFE",
            Self::System    => "SYSTEM",
            Self::User      => "USER",
        }
    }
}

/// Simple deterministic GUID from a seed byte (not cryptographically random —
/// this is an emulator, not real hardware).
fn make_guid(seed: u8) -> [u8; 16] {
    let mut guid = [0u8; 16];
    guid[0] = seed;
    guid[4] = 0x0B; // "ob" for oboromi
    guid[6] = 0x40; // version nibble
    guid[8] = 0x80; // variant
    guid
}

/// Build the default Switch-style GPT partition layout on a 32 GB device.
///
/// Layout (using 4 KiB blocks):
///
/// | Partition | Start Block | Size       |
/// |-----------|-------------|------------|
/// | GPT header| 0           | 34 blocks  |
/// | PRODINFO  | 34          | 8 MiB      |
/// | PRODINFOF | 2082        | 8 MiB      |
/// | SAFE      | 4130        | 64 MiB     |
/// | SYSTEM    | 20514       | 4 GiB      |
/// | USER      | 1069058     | ~27.8 GiB  |
///
/// Block numbers are chosen to align to erase-block boundaries (multiples of 2).
pub fn default_partition_table(total_blocks: u64) -> Vec<GptEntry> {
    let mib = 1024 * 1024 / 4096; // 256 blocks per MiB
    let gib = 1024 * mib;         // 262144 blocks per GiB

    // Reserve first 34 blocks for protective MBR + GPT header + entry array.
    let gpt_reserved = 34;
    // Reserve last 33 blocks for backup GPT.
    let backup_gpt = 33;
    let last_usable = total_blocks - backup_gpt - 1;

    let prodinfo_start  = gpt_reserved;
    let prodinfo_blocks = 8 * mib; // 8 MiB

    let prodinfof_start  = prodinfo_start + prodinfo_blocks;
    let prodinfof_blocks = 8 * mib;

    let safe_start  = prodinfof_start + prodinfof_blocks;
    let safe_blocks = 64 * mib;

    let system_start  = safe_start + safe_blocks;
    let system_blocks = 4 * gib;

    let user_start = system_start + system_blocks;
    // USER gets everything remaining.
    let user_end = last_usable;

    vec![
        GptEntry {
            name: "PRODINFO".into(),
            start_block: prodinfo_start,
            end_block: prodinfo_start + prodinfo_blocks - 1,
            type_guid: make_guid(0x01),
            unique_guid: make_guid(0x11),
        },
        GptEntry {
            name: "PRODINFOF".into(),
            start_block: prodinfof_start,
            end_block: prodinfof_start + prodinfof_blocks - 1,
            type_guid: make_guid(0x02),
            unique_guid: make_guid(0x12),
        },
        GptEntry {
            name: "SAFE".into(),
            start_block: safe_start,
            end_block: safe_start + safe_blocks - 1,
            type_guid: make_guid(0x03),
            unique_guid: make_guid(0x13),
        },
        GptEntry {
            name: "SYSTEM".into(),
            start_block: system_start,
            end_block: system_start + system_blocks - 1,
            type_guid: make_guid(0x04),
            unique_guid: make_guid(0x14),
        },
        GptEntry {
            name: "USER".into(),
            start_block: user_start,
            end_block: user_end,
            type_guid: make_guid(0x05),
            unique_guid: make_guid(0x15),
        },
    ]
}

/// GPT-aware view over an [`EmulatedStorage`] device.
pub struct PartitionTable {
    pub entries: Vec<GptEntry>,
}

impl PartitionTable {
    /// Create a new partition table with the default Switch layout.
    pub fn new_default(total_blocks: u64) -> Self {
        Self {
            entries: default_partition_table(total_blocks),
        }
    }

    /// Look up a partition by its [`PartitionId`].
    pub fn find(&self, id: PartitionId) -> Option<&GptEntry> {
        let name = id.name();
        self.entries.iter().find(|e| e.name == name)
    }

    /// Look up a partition by name (case-insensitive).
    pub fn find_by_name(&self, name: &str) -> Option<&GptEntry> {
        let upper = name.to_uppercase();
        self.entries.iter().find(|e| e.name == upper)
    }

    /// Read `buf.len()` bytes from offset `offset` within the given partition.
    pub fn read_partition(
        &self,
        storage: &mut EmulatedStorage,
        id: PartitionId,
        offset: u64,
        buf: &mut [u8],
    ) -> io::Result<usize> {
        let entry = self.find(id).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, format!("partition {:?} not found", id))
        })?;
        let part_size = entry.size_bytes(storage.block_size());
        if offset + buf.len() as u64 > part_size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "read exceeds partition bounds",
            ));
        }
        let abs_offset = entry.start_block * storage.block_size() + offset;
        storage.read_bytes(abs_offset, buf)
    }

    /// Write `buf` at offset `offset` within the given partition.
    pub fn write_partition(
        &self,
        storage: &mut EmulatedStorage,
        id: PartitionId,
        offset: u64,
        buf: &[u8],
    ) -> io::Result<()> {
        let entry = self.find(id).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, format!("partition {:?} not found", id))
        })?;
        let part_size = entry.size_bytes(storage.block_size());
        if offset + buf.len() as u64 > part_size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "write exceeds partition bounds",
            ));
        }
        let abs_offset = entry.start_block * storage.block_size() + offset;
        storage.write_bytes(abs_offset, buf)
    }
}
