//! Fixed-capacity string interning pool.
//!
//! All strings live in a single flat buffer. Each string is stored as
//! a (offset, length) pair. Deduplication by content.

use crate::VmError;

/// Fixed-capacity string pool.
///
/// - `BYTES`: total byte capacity for string content.
/// - `COUNT`: maximum number of interned strings.
pub struct StringPool<const BYTES: usize, const COUNT: usize> {
    /// Flat buffer holding all string bytes.
    buf: [u8; BYTES],
    /// Next free byte offset in `buf`.
    used: usize,
    /// Entries: (offset, length) pairs.
    entries: [(u16, u16); COUNT],
    /// Number of interned strings.
    len: usize,
}

impl<const BYTES: usize, const COUNT: usize> StringPool<BYTES, COUNT> {
    pub const fn new() -> Self {
        Self {
            buf: [0u8; BYTES],
            used: 0,
            entries: [(0, 0); COUNT],
            len: 0,
        }
    }

    /// Intern a string, returning its index. Deduplicates.
    pub fn intern(&mut self, s: &str) -> Result<u16, VmError> {
        let bytes = s.as_bytes();

        // Check for existing entry
        for i in 0..self.len {
            let (off, len) = self.entries[i];
            if len as usize == bytes.len() {
                let existing = &self.buf[off as usize..off as usize + len as usize];
                if existing == bytes {
                    return Ok(i as u16);
                }
            }
        }

        // Need a new entry
        if self.len >= COUNT {
            return Err(VmError::StringPoolFull);
        }
        if self.used + bytes.len() > BYTES {
            return Err(VmError::StringPoolFull);
        }

        let off = self.used;
        self.buf[off..off + bytes.len()].copy_from_slice(bytes);
        self.entries[self.len] = (off as u16, bytes.len() as u16);
        self.used += bytes.len();
        let idx = self.len as u16;
        self.len += 1;
        Ok(idx)
    }

    /// Get a string by index.
    pub fn get(&self, idx: u16) -> Option<&str> {
        let i = idx as usize;
        if i >= self.len {
            return None;
        }
        let (off, len) = self.entries[i];
        let bytes = &self.buf[off as usize..off as usize + len as usize];
        core::str::from_utf8(bytes).ok()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn clear(&mut self) {
        self.used = 0;
        self.len = 0;
    }
}
