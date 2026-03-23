//! Dictionary — maps word names to program offsets.

use crate::VmError;

/// A single dictionary entry: name → program offset.
#[derive(Clone, Copy)]
struct Entry {
    /// Index into the string pool for the word name.
    name: u16,
    /// Offset into the program where the word body starts.
    offset: u16,
}

/// Fixed-capacity dictionary.
pub struct Dict<const N: usize> {
    entries: [Entry; N],
    len: usize,
}

impl<const N: usize> Dict<N> {
    pub const fn new() -> Self {
        Self {
            entries: [Entry { name: 0, offset: 0 }; N],
            len: 0,
        }
    }

    /// Define a word. Overwrites if the name already exists.
    pub fn define(&mut self, name: u16, offset: u16) -> Result<(), VmError> {
        // Check for redefinition
        for i in 0..self.len {
            if self.entries[i].name == name {
                self.entries[i].offset = offset;
                return Ok(());
            }
        }

        if self.len >= N {
            return Err(VmError::DictFull);
        }

        self.entries[self.len] = Entry { name, offset };
        self.len += 1;
        Ok(())
    }

    /// Look up a word by name string pool index.
    pub fn lookup(&self, name: u16) -> Option<u16> {
        for i in 0..self.len {
            if self.entries[i].name == name {
                return Some(self.entries[i].offset);
            }
        }
        None
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }
}
