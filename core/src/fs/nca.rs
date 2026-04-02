use std::io;
use std::path::Path;

use aes::Aes128;
use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit, generic_array::GenericArray};

use crate::fs::keys::{KeyAreaType, KeySet};

// NCA header is 0xC00 bytes, split into six 0x200-byte XTS sectors.
const HEADER_SIZE: usize = 0xC00;
const SECTOR_SIZE: usize = 0x200;

/// Parsed, decrypted NCA header.
#[derive(Debug, Clone)]
pub struct NcaHeader {
    /// NCA format version ("NCA3", "NCA2", etc.).
    pub magic: [u8; 4],
    /// Distribution type (0 = System, 1 = Gamecard).
    pub distribution_type: u8,
    /// Content type (see [`ContentType`]).
    pub content_type: ContentType,
    /// Key generation (effective max of old + new field).
    pub key_generation: u8,
    /// Which key_area_key table to use (Application/System/Ocean).
    pub key_area_key_index: u8,
    /// Total NCA content size in bytes.
    pub content_size: u64,
    /// Program / Title ID.
    pub program_id: u64,
    /// Rights ID (16 bytes).  All-zeros means not title-key encrypted.
    pub rights_id: [u8; 16],
    /// Up to four section entries.
    pub sections: [NcaSectionEntry; 4],
    /// Decrypted key area (4 × 16 bytes).
    pub key_area: [[u8; 16]; 4],
    /// Section filesystem headers (one per section).
    pub section_headers: [NcaSectionHeader; 4],
}

/// Type of content stored in an NCA.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentType {
    Program,    // ExeFS + RomFS (executables)
    Meta,       // CNMT (content metadata)
    Control,    // Game icon / title info
    Manual,     // HTML manual
    Data,       // Generic data
    PublicData,
    Unknown(u8),
}

impl ContentType {
    fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Program,
            1 => Self::Meta,
            2 => Self::Control,
            3 => Self::Manual,
            4 => Self::Data,
            5 => Self::PublicData,
            x => Self::Unknown(x),
        }
    }
}

/// One entry in the NCA section table.
#[derive(Debug, Clone, Copy, Default)]
pub struct NcaSectionEntry {
    /// Start offset in media units (1 media unit = 0x200 bytes).
    pub media_offset_start: u32,
    /// End offset in media units.
    pub media_offset_end: u32,
    pub enabled: bool,
}

impl NcaSectionEntry {
    /// Byte offset of this section within the NCA file.
    pub fn byte_offset(&self) -> u64 {
        self.media_offset_start as u64 * 0x200
    }
    /// Byte length of this section.
    pub fn byte_size(&self) -> u64 {
        (self.media_offset_end - self.media_offset_start) as u64 * 0x200
    }
}

/// Filesystem type of an NCA section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsType {
    RomFs,
    Pfs0,
    Unknown(u8),
}

/// Hash type used in a section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashType {
    Auto,
    None,
    HierarchicalSha256, // PFS0 sections
    HierarchicalIntegrity, // RomFS (IVFC)
    Unknown(u8),
}

/// Parsed section filesystem header (located at NCA+0x400, 0x600, 0x800, 0xA00).
#[derive(Debug, Clone, Default)]
pub struct NcaSectionHeader {
    pub fs_type: Option<FsType>,
    pub hash_type: Option<HashType>,
    /// For PFS0 sections: offset within the hash region to the PFS0 header.
    pub pfs0_offset: u64,
    /// Size of the PFS0 header (including string table + entries).
    pub pfs0_size: u64,
    pub enabled: bool,
}

impl NcaHeader {
    /// Effective key generation (combines old and new field, takes the max).
    fn key_gen(old: u8, new: u8) -> u8 {
        if new == 0xFF { old } else { new.max(old) }
    }

    /// Parse a decrypted 0xC00-byte header buffer.
    fn from_decrypted(data: &[u8]) -> io::Result<Self> {
        if data.len() < HEADER_SIZE {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "header too short"));
        }

        let magic = [data[0x200], data[0x201], data[0x202], data[0x203]];
        if &magic != b"NCA3" && &magic != b"NCA2" && &magic != b"NCA1" && &magic != b"NCA0" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid NCA magic: {:?}", std::str::from_utf8(&magic)),
            ));
        }

        let distribution_type    = data[0x204];
        let content_type         = ContentType::from_u8(data[0x205]);
        let key_gen_old          = data[0x206];
        let key_area_key_index   = data[0x207];
        let content_size         = u64::from_le_bytes(data[0x208..0x210].try_into().unwrap());
        let program_id           = u64::from_le_bytes(data[0x210..0x218].try_into().unwrap());
        let key_gen_new          = data[0x220];
        let key_generation       = Self::key_gen(key_gen_old, key_gen_new);

        let mut rights_id = [0u8; 16];
        rights_id.copy_from_slice(&data[0x230..0x240]);

        // Section entries (4 × 0x10 bytes at 0x240).
        let mut sections = [NcaSectionEntry::default(); 4];
        for (i, sec) in sections.iter_mut().enumerate() {
            let base = 0x240 + i * 0x10;
            sec.media_offset_start = u32::from_le_bytes(data[base..base+4].try_into().unwrap());
            sec.media_offset_end   = u32::from_le_bytes(data[base+4..base+8].try_into().unwrap());
            sec.enabled = sec.media_offset_start != 0 || sec.media_offset_end != 0;
        }

        // Key area (4 × 16 bytes at 0x300) — still encrypted at this stage.
        let mut key_area_raw = [[0u8; 16]; 4];
        for (i, k) in key_area_raw.iter_mut().enumerate() {
            k.copy_from_slice(&data[0x300 + i * 16..0x300 + (i + 1) * 16]);
        }

        // Section headers at 0x400, 0x600, 0x800, 0xA00.
        let mut section_headers = [
            NcaSectionHeader::default(),
            NcaSectionHeader::default(),
            NcaSectionHeader::default(),
            NcaSectionHeader::default(),
        ];
        for (i, sh) in section_headers.iter_mut().enumerate() {
            let base = 0x400 + i * 0x200;
            if base + 0x200 > data.len() { break; }
            let sec_data = &data[base..base + 0x200];

            // Version (u16) at +0
            let _version  = u16::from_le_bytes([sec_data[0], sec_data[1]]);
            let fs_type   = sec_data[2];
            let hash_type = sec_data[3];

            sh.enabled   = sections[i].enabled;
            sh.fs_type   = Some(match fs_type   { 0 => FsType::RomFs, 1 => FsType::Pfs0, x => FsType::Unknown(x) });
            sh.hash_type = Some(match hash_type  { 0 => HashType::Auto, 1 => HashType::None, 2 => HashType::HierarchicalSha256, 3 => HashType::HierarchicalIntegrity, x => HashType::Unknown(x) });

            // For PFS0 (hash_type = HierarchicalSha256): PFS0 superblock at +0x8.
            if hash_type == 2 {
                // Superblock layout (NCA3): hash_data at +0x8 (0xF8 bytes).
                // Within hash_data: levels[0].offset at +0, levels[0].size at +8.
                // The PFS0 itself is at hash_data + 0x48:
                //   pfs0_offset = levels[2].offset at hash_data+0x38
                //   pfs0_size   = levels[2].size   at hash_data+0x40
                // (layout: two hash levels + the PFS0 level)
                let hd = &sec_data[0x8..];
                // Level 2 (the actual PFS0): offset at hd+0x38, size at hd+0x40
                if hd.len() >= 0x48 {
                    sh.pfs0_offset = u64::from_le_bytes(hd[0x38..0x40].try_into().unwrap());
                    sh.pfs0_size   = u64::from_le_bytes(hd[0x40..0x48].try_into().unwrap());
                }
            }
        }

        Ok(NcaHeader {
            magic,
            distribution_type,
            content_type,
            key_generation,
            key_area_key_index,
            content_size,
            program_id,
            rights_id,
            sections,
            key_area: key_area_raw,
            section_headers,
        })
    }

    /// Whether this NCA uses title-key encryption (rights_id != all zeros).
    pub fn is_title_key_encrypted(&self) -> bool {
        self.rights_id.iter().any(|&b| b != 0)
    }

    /// Decrypt the key area using the appropriate key_area_key from `keys`.
    /// Returns the decrypted key area (4 × 16 bytes).
    pub fn decrypt_key_area(&self, keys: &KeySet) -> Option<[[u8; 16]; 4]> {
        let kak_type = KeyAreaType::from_index(self.key_area_key_index);
        let kak = keys.key_area_key(kak_type, self.key_generation as usize)?;
        let cipher = Aes128::new(GenericArray::from_slice(kak));
        let mut result = self.key_area;
        for key in result.iter_mut() {
            let mut block = GenericArray::from(*key);
            cipher.decrypt_block(&mut block);
            *key = block.into();
        }
        Some(result)
    }
}

/// Open an NCA file, decrypt the header, and return parsed metadata.
pub fn open<P: AsRef<Path>>(path: P, keys: &KeySet) -> io::Result<NcaHeader> {
    let header_key = keys.header_key.ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "header_key missing from key set")
    })?;

    let file_bytes = std::fs::read(path.as_ref())?;
    if file_bytes.len() < HEADER_SIZE {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "file too small to be an NCA"));
    }

    let mut header = file_bytes[..HEADER_SIZE].to_vec();
    xts_decrypt_header(&header_key, &mut header);

    NcaHeader::from_decrypted(&header)
}

/// Read a section's raw bytes from an already-opened NCA file on disk.
///
/// Returns the raw (possibly encrypted) section data.  For key-area-encrypted
/// NCAs you'll need to AES-CTR-decrypt this before parsing.
pub fn read_section<P: AsRef<Path>>(
    path: P,
    entry: &NcaSectionEntry,
) -> io::Result<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path)?;
    f.seek(SeekFrom::Start(entry.byte_offset()))?;
    let mut buf = vec![0u8; entry.byte_size() as usize];
    f.read_exact(&mut buf)?;
    Ok(buf)
}

// -- XTS-AES implementation ---------------------------------------------------

/// Decrypt the NCA header in-place using AES-128-XTS.
///
/// `header_key` is 32 bytes: [0..16] = cipher key, [16..32] = tweak key.
/// The header is divided into six 0x200-byte sectors numbered 0-5.
fn xts_decrypt_header(header_key: &[u8; 32], data: &mut [u8]) {
    let key1 = &header_key[0..16];
    let key2 = &header_key[16..32];

    for sector in 0..6usize {
        let start = sector * SECTOR_SIZE;
        let end   = start + SECTOR_SIZE;
        if end > data.len() { break; }
        xts_decrypt_sector(key1, key2, sector as u64, &mut data[start..end]);
    }
}

/// XTS-AES-128 decryption of a single 0x200-byte sector.
fn xts_decrypt_sector(key1: &[u8], key2: &[u8], sector_index: u64, data: &mut [u8]) {
    let cipher1 = Aes128::new(GenericArray::from_slice(key1));
    let cipher2 = Aes128::new(GenericArray::from_slice(key2));

    // Compute initial tweak: AES_encrypt(key2, sector_index_as_LE_128).
    let mut tweak_input = GenericArray::from([0u8; 16]);
    tweak_input[..8].copy_from_slice(&sector_index.to_le_bytes());
    cipher2.encrypt_block(&mut tweak_input);
    let mut t: [u8; 16] = tweak_input.into();

    for chunk in data.chunks_mut(16) {
        if chunk.len() < 16 { break; }

        // PP = chunk XOR T
        let mut block = [0u8; 16];
        for j in 0..16 { block[j] = chunk[j] ^ t[j]; }

        // AES-128 decrypt
        let mut ga = GenericArray::from(block);
        cipher1.decrypt_block(&mut ga);
        block = ga.into();

        // chunk = block XOR T
        for j in 0..16 { chunk[j] = block[j] ^ t[j]; }

        // Advance tweak: multiply by α in GF(2^128) with polynomial x^128+x^7+x^2+x+1.
        t = gf_mul_alpha(t);
    }
}

/// Multiply a GF(2^128) element (little-endian byte order) by α.
fn gf_mul_alpha(t: [u8; 16]) -> [u8; 16] {
    let carry = (t[15] >> 7) & 1;
    let mut result = [0u8; 16];
    for i in (1..16).rev() {
        result[i] = (t[i] << 1) | (t[i - 1] >> 7);
    }
    result[0] = t[0] << 1;
    if carry != 0 {
        result[0] ^= 0x87;
    }
    result
}
