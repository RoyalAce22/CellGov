//! Multilinear-128 lanes of the runtime's sync-state hash.
//!
//! The sync-state hash reads the committed sync state as one sparse
//! vector of 64-bit lanes. A lane index names a source, an object, a
//! field and a slot:
//!
//! ```text
//! index = source (8 bits) | object (32 bits) | field (8 bits) | slot (15 bits)
//! acc   = key(Y, 0) + sum over nonzero lanes j of key(Y, index_j + 1) * lane_j   (mod 2^128)
//! hash  = acc >> 64
//! ```
//!
//! `key(Y, k)` is [`indexed_key`] over
//! [`SYNC_KEY_SEED`]. The key index `index + 1` stays below 2^63, where
//! the key stream repeats, so no lane shares a key with another lane or
//! with the additive key: the one tuple whose index is 2^63 - 1 does not
//! pack. A zero lane adds nothing, so an absent object and an empty
//! table contribute nothing with no special case.
//!
//! [LemireKaser2014 p:4 s:3] Theorem 3.1 with K = 128 and L = 64 bounds a
//! collision between two distinct lane vectors at 2^-64 over the key
//! draw, and [LemireKaser2014 p:2 s:1] the top 64 bits keep the bound.
//! The keys are fixed, and the bound holds while no hash value steers
//! which states the runtime produces [CarterWegman1979 p:147 s:Properties
//! of Universal Classes].
//!
//! Each table keeps a **partial**: the sum of its own `key * lane` terms.
//! Sources own disjoint index ranges through their tags, so the sum of
//! the partials is one Multilinear hash over the union of the ranges.
//! A table stores its entries in a [`LaneMap`], which keeps the partial
//! on every change; the table defines only the lanes of one value.
//!
//! Two kinds of lane rest on the structural gate, not on the theorem:
//!
//! - A lane whose object id needs more than 32 bits or whose slot needs
//!   more than 15 bits has no packed index. It contributes a mixer term
//!   over the whole component tuple and the value.
//! - Byte content (paths, names, data blobs) has no integer index. Each
//!   entry contributes one mixer term over its tag, its key words and
//!   its length-prefixed bytes.
//!
//! A mixer term is a Multilinear digest of the input words under
//! [`DIGEST_KEY_SEED`], put through the SplitMix64 finalizer, and added
//! into the high half of the sum. The sum adds the terms and does not
//! combine them by XOR [Clarke2003 p:6 s:4]; the key is part of each term, so two
//! entries cannot stand in for each other [Lewi2019 p:8 s:2.1]; and an
//! update costs only the entries it changes [Lewi2019 p:5 s:1.2].

use std::collections::BTreeMap;

use crate::hash::{indexed_key, splitmix64_mix};

/// The SplitMix64 seed of the sync-state lane keys.
pub const SYNC_KEY_SEED: u64 = 0x7379_6e63_7374_6174;

/// The SplitMix64 seed of the mixer-term digest keys.
pub const DIGEST_KEY_SEED: u64 = 0x7379_6e63_6469_6773;

/// Source tags. Each source of the sync state owns the index range of
/// its tag; a tag is never reused for another source.
pub mod source {
    /// Mailbox registry.
    pub const MAILBOX: u8 = 1;
    /// Signal-notification registry.
    pub const SIGNAL: u8 = 2;
    /// Atomic reservation tables, one slot per address space.
    pub const RESERVATION: u8 = 3;
    /// Pending syscall responses, one object per unit.
    pub const SYSCALL_RESPONSE: u8 = 4;
    /// Timer wakes, one object per queue sequence number.
    pub const TIMER_WAKE: u8 = 5;
    /// The RSX FIFO cursor.
    pub const RSX_CURSOR: u8 = 6;
    /// The RSX flip state.
    pub const RSX_FLIP: u8 = 7;
    /// The RSX semaphore offset.
    pub const RSX_SEM_OFFSET: u8 = 8;
    /// Unit-to-address-space tags, one object per unit.
    pub const SPACE_TAG: u8 = 9;
    /// Shared segments, one object per segment in key order.
    pub const SPACE_SHARED: u8 = 10;
    /// The sources that no partial covers yet, folded as one value.
    pub const TRANSITIONAL: u8 = 255;
}

const OBJECT_BITS: u32 = 32;
const FIELD_BITS: u32 = 8;
const SLOT_BITS: u32 = 15;

/// The largest 63-bit index; `index + 1` would be 2^63.
const ALL_ONES: u64 = (1 << (8 + OBJECT_BITS + FIELD_BITS + SLOT_BITS)) - 1;

/// The components of one lane index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaneIndex {
    /// The source tag, from [`source`].
    pub source: u8,
    /// The object id within the source.
    pub object: u64,
    /// The field of the object.
    pub field: u8,
    /// The slot within the field, 0 for a scalar field.
    pub slot: u64,
}

impl LaneIndex {
    /// The index of `field` / `slot` of `object` in `source`.
    #[inline]
    pub const fn new(source: u8, object: u64, field: u8, slot: u64) -> Self {
        Self {
            source,
            object,
            field,
            slot,
        }
    }

    /// The packed 63-bit index, or `None` when the object or the slot
    /// does not fit its field, or when the index is the all-ones value
    /// whose key would be the additive key.
    #[inline]
    pub const fn packed(self) -> Option<u64> {
        if self.object >> OBJECT_BITS != 0 || self.slot >> SLOT_BITS != 0 {
            return None;
        }
        let index = ((self.source as u64) << (OBJECT_BITS + FIELD_BITS + SLOT_BITS))
            | (self.object << (FIELD_BITS + SLOT_BITS))
            | ((self.field as u64) << SLOT_BITS)
            | self.slot;
        if index == ALL_ONES {
            return None;
        }
        Some(index)
    }
}

/// The additive key `key(Y, 0)`, the sum of an empty lane vector.
#[inline]
pub fn additive_key() -> u128 {
    indexed_key(SYNC_KEY_SEED, 0)
}

/// The term a lane adds to the sum: `key(Y, index + 1) * value`, 0 for
/// a zero value, or a mixer term when the index does not pack.
#[inline]
pub fn contribution(index: LaneIndex, value: u64) -> u128 {
    if value == 0 {
        return 0;
    }
    match index.packed() {
        Some(k) => indexed_key(SYNC_KEY_SEED, k + 1).wrapping_mul(u128::from(value)),
        None => mixer_term(&[
            u64::from(index.source),
            index.object,
            u64::from(index.field),
            index.slot,
            value,
        ]),
    }
}

/// The mixer term of one byte-content entry: `tag` and `key` name the
/// entry, `bytes` is its content.
pub fn bytes_term(tag: u8, key: &[u64], bytes: &[u8]) -> u128 {
    let mut digest = Digest::new();
    digest.push(u64::from(tag) | (1 << 8));
    digest.push(key.len() as u64);
    for &word in key {
        digest.push(word);
    }
    digest.push(bytes.len() as u64);
    for chunk in bytes.chunks(8) {
        let mut word = [0u8; 8];
        word[..chunk.len()].copy_from_slice(chunk);
        digest.push(u64::from_le_bytes(word));
    }
    digest.term()
}

/// The mixer term of a word tuple.
fn mixer_term(words: &[u64]) -> u128 {
    let mut digest = Digest::new();
    digest.push(words.len() as u64);
    for &word in words {
        digest.push(word);
    }
    digest.term()
}

/// A Multilinear-128 digest of a word sequence under [`DIGEST_KEY_SEED`].
struct Digest {
    acc: u128,
    next: u64,
}

impl Digest {
    fn new() -> Self {
        Self {
            acc: indexed_key(DIGEST_KEY_SEED, 0),
            next: 1,
        }
    }

    fn push(&mut self, word: u64) {
        self.acc = self
            .acc
            .wrapping_add(indexed_key(DIGEST_KEY_SEED, self.next).wrapping_mul(u128::from(word)));
        self.next += 1;
    }

    /// The finalized digest, placed in the high half of the sum.
    fn term(self) -> u128 {
        u128::from(splitmix64_mix((self.acc >> 64) as u64)) << 64
    }
}

/// The lanes of one object, summed as they are added.
#[derive(Debug, Clone, Copy)]
pub struct ObjectLanes {
    source: u8,
    object: u64,
    slot_base: u64,
    sum: u128,
}

impl ObjectLanes {
    /// Add lane `field` / `slot` with `value`. The lane sits at the
    /// map's slot base plus `slot`.
    #[inline]
    pub fn lane(&mut self, field: u8, slot: u64, value: u64) {
        self.sum = self.sum.wrapping_add(contribution(
            LaneIndex::new(
                self.source,
                self.object,
                field,
                self.slot_base.wrapping_add(slot),
            ),
            value,
        ));
    }
}

/// A value a [`LaneMap`] stores.
pub trait LaneValue {
    /// Add the value's lanes to `lanes`. Fields start at 1: the map adds
    /// the presence lane, field 0, itself.
    fn lanes(&self, lanes: &mut ObjectLanes);
}

/// Where the lanes of a map's entries sit.
struct Shape<K> {
    source: u8,
    slot_base: u64,
    object_of: fn(K) -> u64,
}

impl<K> Clone for Shape<K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K> Copy for Shape<K> {}

impl<K> core::fmt::Debug for Shape<K> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Shape")
            .field("source", &self.source)
            .field("slot_base", &self.slot_base)
            .finish()
    }
}

impl<K> Shape<K> {
    /// The contribution of the entry `key` -> `value`: its presence lane
    /// and the value's own lanes.
    fn term<V: LaneValue>(self, key: K, value: &V) -> u128 {
        let mut lanes = ObjectLanes {
            source: self.source,
            object: (self.object_of)(key),
            slot_base: self.slot_base,
            sum: 0,
        };
        lanes.lane(0, 0, 1);
        value.lanes(&mut lanes);
        lanes.sum
    }
}

/// An ordered map that keeps its partial of the sync-state sum.
///
/// Each entry contributes a presence lane and the lanes of its
/// [`LaneValue`], with the object id `object_of(key)` in the map's
/// source. Every method that changes an entry updates the partial: it
/// subtracts the entry's old contribution and adds the new one
/// [WegmanCarter1981 p:277 s:5]. [`Self::get_mut`] returns a guard that
/// does the same when it drops, so no path changes an entry without the
/// partial.
#[derive(Debug, Clone)]
pub struct LaneMap<K, V> {
    entries: BTreeMap<K, V>,
    shape: Shape<K>,
    partial: u128,
}

impl<K: Ord + Copy, V: LaneValue> LaneMap<K, V> {
    /// An empty map of `source` whose entry `key` is object
    /// `object_of(key)`.
    pub fn new(source: u8, object_of: fn(K) -> u64) -> Self {
        Self {
            entries: BTreeMap::new(),
            shape: Shape {
                source,
                slot_base: 0,
                object_of,
            },
            partial: 0,
        }
    }

    /// The same map with every lane moved `slot_base` slots up. Two
    /// maps of one source with different slot bases occupy disjoint
    /// lanes while their values use slot 0 only.
    pub fn with_slot_base(mut self, slot_base: u64) -> Self {
        debug_assert!(self.entries.is_empty(), "slot base set on a filled map");
        self.shape.slot_base = slot_base;
        self
    }

    /// Number of entries.
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the map holds no entry.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Borrow the value at `key`.
    #[inline]
    pub fn get(&self, key: K) -> Option<&V> {
        self.entries.get(&key)
    }

    /// Whether the map holds `key`.
    #[inline]
    pub fn contains_key(&self, key: K) -> bool {
        self.entries.contains_key(&key)
    }

    /// Iterate the entries in key order.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = (K, &V)> + '_ {
        self.entries.iter().map(|(k, v)| (*k, v))
    }

    /// The entry with the smallest key.
    #[inline]
    pub fn first(&self) -> Option<(K, &V)> {
        self.entries.first_key_value().map(|(k, v)| (*k, v))
    }

    /// Insert `value` at `key`, returning the value it replaces.
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        self.partial = self.partial.wrapping_add(self.shape.term(key, &value));
        let prior = self.entries.insert(key, value);
        if let Some(old) = &prior {
            self.partial = self.partial.wrapping_sub(self.shape.term(key, old));
        }
        prior
    }

    /// Remove the entry at `key`, returning its value.
    pub fn remove(&mut self, key: K) -> Option<V> {
        let removed = self.entries.remove(&key)?;
        self.partial = self.partial.wrapping_sub(self.shape.term(key, &removed));
        Some(removed)
    }

    /// Remove the entry with the smallest key.
    pub fn pop_first(&mut self) -> Option<(K, V)> {
        let (key, value) = self.entries.pop_first()?;
        self.partial = self.partial.wrapping_sub(self.shape.term(key, &value));
        Some((key, value))
    }

    /// Remove every entry.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.partial = 0;
    }

    /// Mutably borrow the value at `key` through a guard that updates the
    /// partial when it drops.
    pub fn get_mut(&mut self, key: K) -> Option<LaneEntryMut<'_, K, V>> {
        let shape = self.shape;
        let value = self.entries.get_mut(&key)?;
        self.partial = self.partial.wrapping_sub(shape.term(key, &*value));
        Some(LaneEntryMut {
            key,
            value,
            partial: &mut self.partial,
            shape,
        })
    }

    /// Keep the entries for which `keep` returns true. `keep` may change
    /// a value it keeps.
    pub fn retain(&mut self, mut keep: impl FnMut(K, &mut V) -> bool) {
        let shape = self.shape;
        let partial = &mut self.partial;
        self.entries.retain(|key, value| {
            *partial = partial.wrapping_sub(shape.term(*key, &*value));
            let kept = keep(*key, value);
            if kept {
                *partial = partial.wrapping_add(shape.term(*key, &*value));
            }
            kept
        });
    }

    /// The map's partial of the sync-state sum.
    pub fn partial(&self) -> u128 {
        debug_assert_eq!(
            self.partial,
            self.partial_from_scratch(),
            "lane map partial out of date"
        );
        self.partial
    }

    /// [`Self::partial`] computed from every entry.
    pub fn partial_from_scratch(&self) -> u128 {
        self.entries.iter().fold(0u128, |acc, (key, value)| {
            acc.wrapping_add(self.shape.term(*key, value))
        })
    }
}

/// A mutable borrow of one [`LaneMap`] value. The entry's contribution
/// leaves the partial when the guard is made and returns, recomputed,
/// when it drops.
pub struct LaneEntryMut<'a, K: Copy, V: LaneValue> {
    key: K,
    value: &'a mut V,
    partial: &'a mut u128,
    shape: Shape<K>,
}

impl<K: Copy, V: LaneValue> core::ops::Deref for LaneEntryMut<'_, K, V> {
    type Target = V;

    #[inline]
    fn deref(&self) -> &V {
        self.value
    }
}

impl<K: Copy, V: LaneValue> core::ops::DerefMut for LaneEntryMut<'_, K, V> {
    #[inline]
    fn deref_mut(&mut self) -> &mut V {
        self.value
    }
}

impl<K: Copy, V: LaneValue> Drop for LaneEntryMut<'_, K, V> {
    fn drop(&mut self) {
        *self.partial = self
            .partial
            .wrapping_add(self.shape.term(self.key, &*self.value));
    }
}

#[cfg(test)]
#[path = "tests/lanes_tests.rs"]
mod tests;
