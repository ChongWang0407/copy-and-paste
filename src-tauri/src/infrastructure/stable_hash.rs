//! A hasher whose output never changes between Rust releases.
//!
//! Content hashes and attachment file names (`img_<hash>.png`) are stored, so
//! they must stay the same across app updates. `DefaultHasher` doesn't promise
//! that. This is SipHash-1-3 with zero keys, which is what `DefaultHasher`
//! computes today, frozen here (the test pins the two together), so existing
//! hashes and file names keep matching.

use std::hash::Hasher;

pub struct StableHasher {
    v0: u64,
    v1: u64,
    v2: u64,
    v3: u64,
    tail: u64,
    ntail: usize,
    length: usize,
}

impl Default for StableHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl StableHasher {
    pub fn new() -> Self {
        // Keys k0 = k1 = 0.
        Self {
            v0: 0x736f6d6570736575,
            v1: 0x646f72616e646f6d,
            v2: 0x6c7967656e657261,
            v3: 0x7465646279746573,
            tail: 0,
            ntail: 0,
            length: 0,
        }
    }

    #[inline]
    fn round(&mut self) {
        self.v0 = self.v0.wrapping_add(self.v1);
        self.v1 = self.v1.rotate_left(13);
        self.v1 ^= self.v0;
        self.v0 = self.v0.rotate_left(32);
        self.v2 = self.v2.wrapping_add(self.v3);
        self.v3 = self.v3.rotate_left(16);
        self.v3 ^= self.v2;
        self.v0 = self.v0.wrapping_add(self.v3);
        self.v3 = self.v3.rotate_left(21);
        self.v3 ^= self.v0;
        self.v2 = self.v2.wrapping_add(self.v1);
        self.v1 = self.v1.rotate_left(17);
        self.v1 ^= self.v2;
        self.v2 = self.v2.rotate_left(32);
    }

    #[inline]
    fn compress(&mut self, m: u64) {
        self.v3 ^= m;
        self.round(); // c = 1
        self.v0 ^= m;
    }
}

impl Hasher for StableHasher {
    fn write(&mut self, bytes: &[u8]) {
        self.length += bytes.len();
        let mut rest = bytes;
        if self.ntail != 0 {
            while self.ntail < 8 {
                let Some((&byte, after)) = rest.split_first() else {
                    return;
                };
                self.tail |= (byte as u64) << (8 * self.ntail);
                self.ntail += 1;
                rest = after;
            }
            let m = self.tail;
            self.compress(m);
            self.tail = 0;
            self.ntail = 0;
        }
        let mut chunks = rest.chunks_exact(8);
        for chunk in &mut chunks {
            self.compress(u64::from_le_bytes(chunk.try_into().expect("8-byte chunk")));
        }
        for (i, &byte) in chunks.remainder().iter().enumerate() {
            self.tail |= (byte as u64) << (8 * i);
        }
        self.ntail = chunks.remainder().len();
    }

    // Integers and the `[u8]`/`str` length prefix go through `write` as their
    // little-endian bytes, as std's SipHasher13 does on little-endian targets.
    fn write_usize(&mut self, value: usize) {
        self.write(&value.to_le_bytes());
    }

    fn finish(&self) -> u64 {
        let mut state = Self {
            v0: self.v0,
            v1: self.v1,
            v2: self.v2,
            v3: self.v3,
            tail: 0,
            ntail: 0,
            length: 0,
        };
        let b = ((self.length as u64 & 0xff) << 56) | self.tail;
        state.compress(b);
        state.v2 ^= 0xff;
        for _ in 0..3 {
            state.round(); // d = 3
        }
        state.v0 ^ state.v1 ^ state.v2 ^ state.v3
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hash::Hash;

    fn both<T: Hash + ?Sized>(value: &T) -> (u64, u64) {
        let mut stable = StableHasher::new();
        value.hash(&mut stable);
        #[allow(deprecated)]
        let mut default = std::collections::hash_map::DefaultHasher::new();
        value.hash(&mut default);
        (stable.finish(), default.finish())
    }

    #[test]
    fn matches_default_hasher_today() {
        let image: Vec<u8> = (0..10_000u32).map(|i| (i * 31 % 251) as u8).collect();
        for len in [0usize, 1, 7, 8, 9, 15, 16, 17, 63, 64, 65, 1000, 10_000] {
            let (stable, default) = both(&image[..len]);
            assert_eq!(stable, default, "bytes of length {len}");
        }
        let (stable, default) = both("C:\\attachments\\img.png");
        assert_eq!(stable, default);
        let (stable, default) = both(&vec![1u8, 2, 3]);
        assert_eq!(stable, default);
    }

    #[test]
    fn known_value_is_frozen() {
        // Pinned output, so a change in this file can't go unnoticed even if a
        // future Rust changes DefaultHasher and the test above is dropped.
        let mut hasher = StableHasher::new();
        b"tiez".as_slice().hash(&mut hasher);
        assert_eq!(hasher.finish(), include!("stable_hash_vector.txt"));
    }
}
