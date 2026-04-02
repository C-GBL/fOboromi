use std::collections::HashMap;
use std::io;
use std::path::Path;

/// Decoded key set parsed from a `prod.keys` file.
///
/// Key names follow the standard naming convention used by Lockpick_RCM and
/// compatible tools (e.g., `header_key`, `key_area_key_application_00`).
#[derive(Default, Clone)]
pub struct KeySet {
    /// AES-XTS key used to decrypt NCA headers.
    /// 32 bytes: first 16 = cipher key, last 16 = tweak key.
    pub header_key: Option<[u8; 32]>,

    /// Per-generation key_area_key for Application-type NCAs.
    /// Index is key generation (0-based).
    pub key_area_key_application: Vec<[u8; 16]>,

    /// Per-generation key_area_key for System-type NCAs.
    pub key_area_key_system: Vec<[u8; 16]>,

    /// Per-generation key_area_key for Ocean-type NCAs.
    pub key_area_key_ocean: Vec<[u8; 16]>,

    /// Title key encryption keys per generation (used for game NCAs).
    pub title_kek: Vec<[u8; 16]>,

    /// Raw map of every key parsed, for forward-compatibility.
    pub raw: HashMap<String, Vec<u8>>,
}

impl KeySet {
    /// Parse a `prod.keys` (or `title.keys`) file from disk.
    ///
    /// Lines follow the format:
    /// ```text
    /// key_name = <hex_string>
    /// ; comment
    /// ```
    pub fn load<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        Ok(Self::parse(&text))
    }

    /// Parse key file text.
    pub fn parse(text: &str) -> Self {
        let mut ks = KeySet::default();

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            let Some((name, value)) = line.split_once('=') else {
                continue;
            };
            let name = name.trim().to_lowercase();
            let hex = value.trim().replace(' ', "");
            let Ok(bytes) = decode_hex(&hex) else {
                continue;
            };

            ks.raw.insert(name.clone(), bytes.clone());

            match name.as_str() {
                "header_key" => {
                    if bytes.len() == 32 {
                        let mut arr = [0u8; 32];
                        arr.copy_from_slice(&bytes);
                        ks.header_key = Some(arr);
                    }
                }
                n if n.starts_with("key_area_key_application_") => {
                    if let Some(idx) = parse_hex_suffix(n, "key_area_key_application_") {
                        ensure_len(&mut ks.key_area_key_application, idx + 1);
                        if bytes.len() == 16 {
                            ks.key_area_key_application[idx].copy_from_slice(&bytes);
                        }
                    }
                }
                n if n.starts_with("key_area_key_system_") => {
                    if let Some(idx) = parse_hex_suffix(n, "key_area_key_system_") {
                        ensure_len(&mut ks.key_area_key_system, idx + 1);
                        if bytes.len() == 16 {
                            ks.key_area_key_system[idx].copy_from_slice(&bytes);
                        }
                    }
                }
                n if n.starts_with("key_area_key_ocean_") => {
                    if let Some(idx) = parse_hex_suffix(n, "key_area_key_ocean_") {
                        ensure_len(&mut ks.key_area_key_ocean, idx + 1);
                        if bytes.len() == 16 {
                            ks.key_area_key_ocean[idx].copy_from_slice(&bytes);
                        }
                    }
                }
                n if n.starts_with("title_kek_") => {
                    if let Some(idx) = parse_hex_suffix(n, "title_kek_") {
                        ensure_len(&mut ks.title_kek, idx + 1);
                        if bytes.len() == 16 {
                            ks.title_kek[idx].copy_from_slice(&bytes);
                        }
                    }
                }
                _ => {}
            }
        }

        ks
    }

    /// Return the key_area_key for a given type and generation, or None.
    pub fn key_area_key(&self, key_type: KeyAreaType, generation: usize) -> Option<&[u8; 16]> {
        let vec = match key_type {
            KeyAreaType::Application => &self.key_area_key_application,
            KeyAreaType::System      => &self.key_area_key_system,
            KeyAreaType::Ocean       => &self.key_area_key_ocean,
        };
        vec.get(generation)
    }
}

/// Which key_area_key table to use for a given NCA.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAreaType {
    Application,
    System,
    Ocean,
}

impl KeyAreaType {
    pub fn from_index(idx: u8) -> Self {
        match idx {
            0 => Self::Application,
            1 => Self::Ocean,
            2 => Self::System,
            _ => Self::Application,
        }
    }
}

// -- Helpers ------------------------------------------------------------------

fn decode_hex(s: &str) -> Result<Vec<u8>, ()> {
    if s.len() % 2 != 0 {
        return Err(());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| ()))
        .collect()
}

/// Parse the trailing hex number in a key name like `key_area_key_system_0a`.
fn parse_hex_suffix(name: &str, prefix: &str) -> Option<usize> {
    let suffix = name.strip_prefix(prefix)?;
    usize::from_str_radix(suffix, 16).ok()
}

fn ensure_len(v: &mut Vec<[u8; 16]>, len: usize) {
    while v.len() < len {
        v.push([0u8; 16]);
    }
}
