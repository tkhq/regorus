// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use alloc::string::String;
use core::hash::{BuildHasherDefault, Hasher};
use indexmap::IndexMap;

/// Deterministic hasher used by RVM entry point maps.
///
/// Entry point maps are bounded by [`super::Program::MAX_ENTRY_POINTS`] and preserve insertion
/// order. A fixed FNV-1a hasher keeps their construction available in `no_std` builds without
/// introducing randomized hashing.
#[derive(Debug, Clone)]
pub struct EntryPointHasher(u64);

impl Default for EntryPointHasher {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Hasher for EntryPointHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

/// Insertion-ordered entry point map with an explicit `no_std`-compatible hasher.
pub type EntryPointMap = IndexMap<String, usize, BuildHasherDefault<EntryPointHasher>>;
