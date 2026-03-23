//! Fixed-capacity buffer pool for raw byte data (I2C/SPI transfers).
//!
//! Similar to StringPool but for mutable binary data. Each buffer is
//! a (offset, length) region in a flat backing array.

use crate::VmError;

/// Fixed-capacity buffer pool.
///
/// - `BYTES`: total byte capacity.
/// - `COUNT`: maximum number of allocated buffers.
pub struct BufferPool<const BYTES: usize, const COUNT: usize> {
    buf: [u8; BYTES],
    used: usize,
    entries: [(u16, u16); COUNT], // (offset, length)
    len: usize,
}

impl<const BYTES: usize, const COUNT: usize> BufferPool<BYTES, COUNT> {
    pub const fn new() -> Self {
        Self {
            buf: [0u8; BYTES],
            used: 0,
            entries: [(0, 0); COUNT],
            len: 0,
        }
    }

    /// Allocate a new buffer of `size` bytes, zero-initialized.
    /// Returns the buffer index.
    pub fn alloc(&mut self, size: usize) -> Result<u16, VmError> {
        if self.len >= COUNT {
            return Err(VmError::BufferPoolFull);
        }
        if self.used + size > BYTES {
            return Err(VmError::BufferPoolFull);
        }

        let off = self.used;
        // Zero-initialize
        for b in &mut self.buf[off..off + size] {
            *b = 0;
        }
        self.entries[self.len] = (off as u16, size as u16);
        self.used += size;
        let idx = self.len as u16;
        self.len += 1;
        Ok(idx)
    }

    /// Allocate a buffer and copy data into it. Returns the buffer index.
    pub fn alloc_from(&mut self, data: &[u8]) -> Result<u16, VmError> {
        if self.len >= COUNT {
            return Err(VmError::BufferPoolFull);
        }
        if self.used + data.len() > BYTES {
            return Err(VmError::BufferPoolFull);
        }

        let off = self.used;
        self.buf[off..off + data.len()].copy_from_slice(data);
        self.entries[self.len] = (off as u16, data.len() as u16);
        self.used += data.len();
        let idx = self.len as u16;
        self.len += 1;
        Ok(idx)
    }

    /// Get an immutable slice of a buffer by index.
    pub fn get(&self, idx: u16) -> Option<&[u8]> {
        let i = idx as usize;
        if i >= self.len {
            return None;
        }
        let (off, len) = self.entries[i];
        Some(&self.buf[off as usize..off as usize + len as usize])
    }

    /// Get a mutable slice of a buffer by index.
    pub fn get_mut(&mut self, idx: u16) -> Option<&mut [u8]> {
        let i = idx as usize;
        if i >= self.len {
            return None;
        }
        let (off, len) = self.entries[i];
        Some(&mut self.buf[off as usize..off as usize + len as usize])
    }

    /// Get the length of a buffer by index.
    pub fn buf_len(&self, idx: u16) -> Option<usize> {
        let i = idx as usize;
        if i >= self.len {
            return None;
        }
        Some(self.entries[i].1 as usize)
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
