use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Block size for UFS/eMMC emulation (4 KiB, standard for UFS).
pub const BLOCK_SIZE: u64 = 4096;

/// Total capacity: 32 GB.
pub const STORAGE_CAPACITY: u64 = 32 * 1024 * 1024 * 1024;

/// Total number of blocks in the emulated device.
pub const TOTAL_BLOCKS: u64 = STORAGE_CAPACITY / BLOCK_SIZE;

/// Represents an emulated 32 GB UFS/eMMC storage device.
///
/// Backed by a sparse file on the host filesystem so it doesn't consume
/// 32 GB of real disk space — only written blocks occupy physical storage.
pub struct EmulatedStorage {
    file: File,
    path: PathBuf,
    capacity: u64,
    block_size: u64,
    write_protected: bool,
}

impl EmulatedStorage {
    /// Open or create an emulated storage image at `path`.
    ///
    /// If the file doesn't exist it is created as a sparse file sized to
    /// [`STORAGE_CAPACITY`].  An existing file is reused as-is (must be at
    /// least `STORAGE_CAPACITY` bytes or it will be extended).
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();

        // Ensure parent directory exists.
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;

        let meta = file.metadata()?;
        if meta.len() < STORAGE_CAPACITY {
            // Extend to full capacity — the OS keeps it sparse.
            file.set_len(STORAGE_CAPACITY)?;
        }

        Ok(Self {
            file,
            path,
            capacity: STORAGE_CAPACITY,
            block_size: BLOCK_SIZE,
            write_protected: false,
        })
    }

    /// Open the storage image in read-only / write-protected mode.
    pub fn open_read_only<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();

        let file = OpenOptions::new()
            .read(true)
            .write(false)
            .open(&path)?;

        let meta = file.metadata()?;
        if meta.len() < STORAGE_CAPACITY {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "storage image is smaller than 32 GB",
            ));
        }

        Ok(Self {
            file,
            path,
            capacity: STORAGE_CAPACITY,
            block_size: BLOCK_SIZE,
            write_protected: true,
        })
    }

    // -- Info ------------------------------------------------------------------

    /// Total capacity in bytes.
    pub fn capacity(&self) -> u64 {
        self.capacity
    }

    /// Block size in bytes.
    pub fn block_size(&self) -> u64 {
        self.block_size
    }

    /// Total number of blocks.
    pub fn total_blocks(&self) -> u64 {
        self.capacity / self.block_size
    }

    /// Path to the backing image on the host.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the device is write-protected.
    pub fn is_write_protected(&self) -> bool {
        self.write_protected
    }

    /// Set or clear write-protection.
    pub fn set_write_protected(&mut self, wp: bool) {
        self.write_protected = wp;
    }

    // -- Block I/O -------------------------------------------------------------

    /// Read a single block into `buf`.
    ///
    /// `buf` must be exactly [`BLOCK_SIZE`] bytes.  Returns the number of
    /// bytes read (always `BLOCK_SIZE` on success).
    pub fn read_block(&mut self, block_index: u64, buf: &mut [u8]) -> io::Result<usize> {
        if buf.len() as u64 != self.block_size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "buffer length {} does not match block size {}",
                    buf.len(),
                    self.block_size
                ),
            ));
        }
        if block_index >= self.total_blocks() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("block index {} out of range (max {})", block_index, self.total_blocks() - 1),
            ));
        }

        let offset = block_index * self.block_size;
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.read_exact(buf)?;
        Ok(buf.len())
    }

    /// Write a single block from `buf`.
    ///
    /// `buf` must be exactly [`BLOCK_SIZE`] bytes.
    pub fn write_block(&mut self, block_index: u64, buf: &[u8]) -> io::Result<()> {
        if self.write_protected {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "storage is write-protected",
            ));
        }
        if buf.len() as u64 != self.block_size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "buffer length {} does not match block size {}",
                    buf.len(),
                    self.block_size
                ),
            ));
        }
        if block_index >= self.total_blocks() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("block index {} out of range (max {})", block_index, self.total_blocks() - 1),
            ));
        }

        let offset = block_index * self.block_size;
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.write_all(buf)?;
        Ok(())
    }

    /// Read multiple contiguous blocks starting at `start_block`.
    pub fn read_blocks(&mut self, start_block: u64, count: u64, buf: &mut [u8]) -> io::Result<usize> {
        let expected_len = count * self.block_size;
        if buf.len() < expected_len as usize {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "buffer too small for requested block count",
            ));
        }
        if start_block + count > self.total_blocks() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "block range exceeds device capacity",
            ));
        }

        let offset = start_block * self.block_size;
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.read_exact(&mut buf[..expected_len as usize])?;
        Ok(expected_len as usize)
    }

    /// Write multiple contiguous blocks starting at `start_block`.
    pub fn write_blocks(&mut self, start_block: u64, count: u64, buf: &[u8]) -> io::Result<()> {
        if self.write_protected {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "storage is write-protected",
            ));
        }
        let expected_len = count * self.block_size;
        if (buf.len() as u64) < expected_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "buffer too small for requested block count",
            ));
        }
        if start_block + count > self.total_blocks() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "block range exceeds device capacity",
            ));
        }

        let offset = start_block * self.block_size;
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.write_all(&buf[..expected_len as usize])?;
        Ok(())
    }

    // -- Byte-level I/O (convenience) ------------------------------------------

    /// Read `buf.len()` bytes starting at byte offset `offset`.
    pub fn read_bytes(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        if offset + buf.len() as u64 > self.capacity {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "read exceeds device capacity",
            ));
        }

        self.file.seek(SeekFrom::Start(offset))?;
        self.file.read_exact(buf)?;
        Ok(buf.len())
    }

    /// Write `buf` starting at byte offset `offset`.
    pub fn write_bytes(&mut self, offset: u64, buf: &[u8]) -> io::Result<()> {
        if self.write_protected {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "storage is write-protected",
            ));
        }
        if offset + buf.len() as u64 > self.capacity {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "write exceeds device capacity",
            ));
        }

        self.file.seek(SeekFrom::Start(offset))?;
        self.file.write_all(buf)?;
        Ok(())
    }

    /// Flush any buffered writes to the backing file.
    pub fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}
