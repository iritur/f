// SPDX-License-Identifier: Apache-2.0 OR MIT
//! One entry in the index log: a namespace, a name, and what it names.
//!
//! # No byte from the device is a field until [`Entry::from_slice`] returned `Ok`
//!
//! The same rule `blob/src/store.rs` is arranged around, for the same reason and
//! against the same peer: somebody else wrote these bytes, possibly across a
//! power cut. The magic, then the declared length, then the reserved word, then
//! the namespace and the operation — refused in that order, before any of them
//! is handed back. A length field a device wrote is never used to slice
//! anything until it has been believed.
//!
//! # Why the padding test is separate from the decode
//!
//! Because *a zeroed block is the end of the log* and *these bytes are not a
//! record* are two different statements and only one of them is a fault. A
//! decoder that treated a refusal as end-of-log would read a torn record as a
//! clean end and silently drop everything after it. So [`padding_at`] answers
//! the first question — the four magic bytes are zero, which an unformatted or
//! never-written block always is — and anything that is not padding must decode
//! or be refused. R04, and it is the whole difference between a truncated index
//! and a corrupt one.

use f_abi::store::refusal;

/// The entry's magic. `F_IX`, little-endian.
///
/// A magic of its own rather than the store's, on `abi/src/store.rs`'s stated
/// grounds: these records are found at addresses a caller computed — a log block
/// this crate counted to — and a wrong address lands on a plausible record far
/// more often than it lands on nothing. A fourth magic makes that arithmetic
/// error a refusal rather than a misreading.
/// Unit: none — a fixed byte pattern.
pub const MAGIC: u32 = 0x5849_5f46;

/// How many bytes of every entry precede its name.
///
/// Unit: bytes.
pub const HEAD_BYTES: usize = 44;

/// The longest name this format can hold.
///
/// Unit: bytes. Chosen so that [`ENTRY_MAX_BYTES`] fits inside the smallest
/// block `f-blob` will format a device with (512), because **no entry spans two
/// blocks**: a record split across a block boundary could not have its magic
/// believed until both blocks had been read, which is a reader that reads twice
/// before it disbelieves anything. It bounds a path, and a path this tree cannot
/// name is refused rather than truncated.
///
/// *What would reverse this:* a device whose logical block is 512 and a caller
/// with a longer path than 448 bytes. The fix is then a continuation entry and
/// not a wider bound, because the bound is the block and not the taste.
pub const NAME_MAX: usize = 448;

/// The widest an entry can encode to.
///
/// Unit: bytes. A mount refuses a device whose block cannot hold one, which is
/// the check that makes the no-spanning rule true rather than intended.
pub const ENTRY_MAX_BYTES: usize = HEAD_BYTES + NAME_MAX;

/// The namespaces a name is qualified by, and a fifth is an RFC.
///
/// A module of constants rather than an `enum`, which is this tree's shape
/// wherever a value arrives from a peer: an `enum` makes the *known* values a
/// type and leaves every decoder to invent its own answer for an unknown one.
/// [`ns::known`] is the one answer, and it is `false` for zero, because a zeroed
/// block must never decode as an entry.
pub mod ns {
    /// A path to the hash of the object at it. The namespace the exit is
    /// measured on.
    /// Unit: none — a namespace identifier, not a quantity.
    pub const PATH: u8 = 1;

    /// Metadata about a name: the key is the name it is about, and the value is
    /// the hash of a blob holding it. Metadata is not stored *in* the index for
    /// the reason the whole design rests on — the index maps names to content
    /// addresses, and content lives in the store, so an index that held bytes
    /// would be a second store with no verifier in it.
    /// Unit: none — a namespace identifier, not a quantity.
    pub const META: u8 = 2;

    /// A declared semantic attribute — what a caller asserted about a thing,
    /// rather than what the thing contains. Separate from [`META`] because the
    /// two answer to different authorities: metadata is derived from the object,
    /// an attribute is claimed by whoever published it, and a namespace that
    /// mixed them would make provenance a matter of reading the key.
    /// Unit: none — a namespace identifier, not a quantity.
    pub const ATTR: u8 = 3;

    /// RFC 0059's pin namespace: a name for a generation root hash, durable, and
    /// the thing the collector's pinned set `P` is read out of.
    ///
    /// Reserved to that RFC. It is a namespace and not a flag, a field or a bit
    /// in a root record for the three reasons that RFC gives — a pin held by a
    /// channel dies with its holder, a bit in a sequential-write-required zone
    /// can never be cleared, and a pin in the state tree is a control plane
    /// reached by writing to a mapping. It is also what makes a pinned
    /// generation survive the root-zone wrap: a pin is a name, and a name is not
    /// a record in a zone that wraps.
    /// Unit: none — a namespace identifier, not a quantity.
    pub const PIN: u8 = 4;

    /// Is this a namespace this build knows?
    #[must_use]
    pub const fn known(value: u8) -> bool {
        matches!(value, PATH | META | ATTR | PIN)
    }

    /// A word for a log or a rendering.
    #[must_use]
    pub const fn label(value: u8) -> &'static str {
        match value {
            PATH => "path",
            META => "meta",
            ATTR => "attr",
            PIN => "pin",
            _ => "unknown",
        }
    }
}

/// What an entry does to the map when it is replayed.
///
/// Two operations and not one, because a log-structured store has no way to
/// express *forget this name* other than by writing it down. `UNPIN` is the
/// caller this was written for: RFC 0059 needs a pin to be removable, and a
/// removal that only happened in memory would come back at the next mount.
pub mod op {
    /// This name now means this hash. The last `SET` for a name wins, which is
    /// what makes a log an update as well as an insert.
    /// Unit: none — an operation identifier, not a quantity.
    pub const SET: u8 = 1;

    /// This name means nothing. A tombstone: the entry is appended like any
    /// other and the space the old entry occupied is reclaimed by nothing here —
    /// compaction is a rewrite of the log, and a rewrite is `f-zone`'s
    /// copy-forward rather than this crate's.
    /// Unit: none — an operation identifier, not a quantity.
    pub const REMOVE: u8 = 2;

    /// Is this an operation this build knows?
    #[must_use]
    pub const fn known(value: u8) -> bool {
        matches!(value, SET | REMOVE)
    }
}

/// One record in the log.
///
/// The name borrows rather than owning, on both sides of the codec: encoding
/// takes a caller's slice and decoding hands back a window into the block that
/// was read. A record type that owned its name would allocate once per entry at
/// mount, which on a log of a million entries is a million allocations bought
/// for a lifetime nobody needed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry<'a> {
    /// Which namespace the name is in.
    /// Unit: none — an [`ns`] constant. Zero is not a namespace.
    pub namespace: u8,
    /// What this entry does to the name.
    /// Unit: none — an [`op`] constant. Zero is not an operation.
    pub op: u8,
    /// The content address the name means, or zeros for [`op::REMOVE`].
    ///
    /// Zeros and not an `Option`, because the encoding has a fixed width and a
    /// missing field would have to be signalled by something a peer could get
    /// wrong. A reader that has believed [`Entry::op`] already knows whether
    /// this field means anything.
    /// Unit: none — a SHA-256 content address, not a quantity.
    pub hash: [u8; 32],
    /// The name, in the namespace above.
    /// Unit: bytes — an opaque key, never interpreted here. A path's separators
    /// are the caller's business: this crate compares names and does not parse
    /// them, which is what keeps one codec serving paths, metadata keys and
    /// attributes.
    pub name: &'a [u8],
}

/// How many bytes an entry with this name encodes to.
///
/// # Errors
///
/// [`refusal::MALFORMED`] for an empty name — a key that is nothing is not a
/// name — or one over [`NAME_MAX`], which is a record this format cannot hold.
pub const fn entry_bytes(name_bytes: usize) -> Result<usize, i32> {
    if name_bytes == 0 || name_bytes > NAME_MAX {
        return Err(refusal::MALFORMED);
    }
    Ok(HEAD_BYTES + name_bytes)
}

/// Are the four bytes at `at` zero — that is, is this where the log stops?
///
/// A separate question from *do these bytes decode*, and the separation is the
/// point: see this module's own documentation. A slice too short to hold a magic
/// is padding too, because nothing can start there.
#[must_use]
pub fn padding_at(raw: &[u8], at: usize) -> bool {
    match at.checked_add(4).and_then(|end| raw.get(at..end)) {
        Some(four) => four == [0u8; 4],
        None => true,
    }
}

impl<'a> Entry<'a> {
    /// How many bytes this entry encodes to.
    ///
    /// Unit: bytes.
    ///
    /// # Errors
    ///
    /// As [`entry_bytes`].
    pub const fn encoded_bytes(&self) -> Result<usize, i32> {
        entry_bytes(self.name.len())
    }

    /// Encode into `out`, which must be exactly [`Entry::encoded_bytes`] long.
    ///
    /// Little-endian, field by field, nothing skipped and nothing padded — the
    /// padding in this format is between entries and belongs to the block, not
    /// to the record.
    ///
    /// # Errors
    ///
    /// [`refusal::MALFORMED`] for a name this format cannot hold or an `out`
    /// that is not exactly the encoded width; [`refusal::UNKNOWN`] for a
    /// namespace or an operation this build does not know, which is R04 applied
    /// to the *encoder* as well as the decoder: a caller that invented a
    /// namespace learns it here rather than at somebody else's mount.
    pub fn to_slice(&self, out: &mut [u8]) -> Result<(), i32> {
        let bytes = self.encoded_bytes()?;
        if out.len() != bytes {
            return Err(refusal::MALFORMED);
        }
        if !ns::known(self.namespace) || !op::known(self.op) {
            return Err(refusal::UNKNOWN);
        }
        out[0..4].copy_from_slice(&MAGIC.to_le_bytes());
        out[4..6].copy_from_slice(&(bytes as u16).to_le_bytes());
        out[6] = self.namespace;
        out[7] = self.op;
        out[8..40].copy_from_slice(&self.hash);
        out[40..42].copy_from_slice(&(self.name.len() as u16).to_le_bytes());
        // Reserved, and zero. Two bytes now rather than a migration later: this
        // record has been written to a device the moment anything formats one.
        out[42..44].copy_from_slice(&0u16.to_le_bytes());
        out[HEAD_BYTES..bytes].copy_from_slice(self.name);
        Ok(())
    }

    /// Decode the entry at the start of `raw`, refusing before any field is
    /// handed back.
    ///
    /// `raw` may be longer than the entry — it is the remainder of a block — and
    /// the entry's own declared length says where it ends. That length is
    /// checked against the name length and against what is actually there before
    /// either is used to slice.
    ///
    /// # Errors
    ///
    /// [`refusal::MALFORMED`] for the magic, a declared length that disagrees
    /// with the name length, a name outside its bounds, or an entry that runs
    /// past the bytes given; [`refusal::UNKNOWN`] for a non-zero reserved word,
    /// a namespace or an operation this build does not know — refused and never
    /// skipped, because a reader that skips a record it cannot describe is a
    /// reader that disagrees with the writer about what the log says.
    pub fn from_slice(raw: &'a [u8]) -> Result<Self, i32> {
        if raw.len() < HEAD_BYTES {
            return Err(refusal::MALFORMED);
        }
        if u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) != MAGIC {
            return Err(refusal::MALFORMED);
        }
        let declared = u16::from_le_bytes([raw[4], raw[5]]) as usize;
        let name_bytes = u16::from_le_bytes([raw[40], raw[41]]) as usize;
        if declared != entry_bytes(name_bytes)? || declared > raw.len() {
            return Err(refusal::MALFORMED);
        }
        if u16::from_le_bytes([raw[42], raw[43]]) != 0 {
            return Err(refusal::UNKNOWN);
        }
        let namespace = raw[6];
        let op = raw[7];
        if !ns::known(namespace) || !op::known(op) {
            return Err(refusal::UNKNOWN);
        }
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&raw[8..40]);
        Ok(Self { namespace, op, hash, name: &raw[HEAD_BYTES..declared] })
    }
}

#[cfg(test)]
mod tests {
    use super::{ENTRY_MAX_BYTES, Entry, HEAD_BYTES, MAGIC, NAME_MAX, ns, op, padding_at};
    use alloc::vec;
    use f_abi::store::refusal;

    /// The magic spells `F_IX` when a device is dumped, which is the property a
    /// person reading a hex dump actually uses and the one a transposed constant
    /// would break silently.
    #[test]
    fn the_magic_is_four_readable_bytes() {
        assert_eq!(MAGIC.to_le_bytes(), *b"F_IX");
    }

    /// The no-spanning rule is arithmetic and not a hope: the widest entry fits
    /// in the smallest block this tree formats a device with.
    #[test]
    fn the_widest_entry_fits_in_the_smallest_block() {
        assert!(ENTRY_MAX_BYTES <= f_blob::store::MIN_BLOCK_BYTES as usize);
    }

    #[test]
    fn what_was_encoded_is_what_is_decoded() {
        let name = b"docs/design/store.html";
        let entry = Entry { namespace: ns::PATH, op: op::SET, hash: [7u8; 32], name };
        let mut out = vec![0u8; entry.encoded_bytes().expect("a name this format can hold")];
        entry.to_slice(&mut out).expect("an entry this format can write");
        assert_eq!(Entry::from_slice(&out), Ok(entry));
        // And with the remainder of a block after it, which is how it is really
        // read: the declared length and not the slice length is what ends it.
        let mut block = vec![0u8; 512];
        block[..out.len()].copy_from_slice(&out);
        assert_eq!(Entry::from_slice(&block), Ok(entry));
    }

    #[test]
    fn a_zeroed_block_is_padding_and_not_a_refusal() {
        let block = vec![0u8; 512];
        assert!(padding_at(&block, 0));
        assert!(padding_at(&block, 509), "fewer than four bytes left is padding too");
        // And it is *not* a decodable entry, which is the other half: padding is
        // where the log stops, and stopping is a caller's decision rather than
        // something the codec does on its behalf.
        assert_eq!(Entry::from_slice(&block), Err(refusal::MALFORMED));
    }

    /// A torn record is refused rather than read as the end of the log. This is
    /// the case the separate padding test exists for, and without it a single
    /// corrupt byte would silently truncate an index.
    #[test]
    fn a_record_that_is_not_padding_and_not_a_record_is_refused() {
        let name = b"a";
        let entry = Entry { namespace: ns::PATH, op: op::SET, hash: [0u8; 32], name };
        let mut out = vec![0u8; entry.encoded_bytes().expect("one byte is a name")];
        entry.to_slice(&mut out).expect("an entry this format can write");

        let mut torn = out.clone();
        torn[0] ^= 1;
        assert!(!padding_at(&torn, 0));
        assert_eq!(Entry::from_slice(&torn), Err(refusal::MALFORMED));

        // A declared length that disagrees with the name length, which is the
        // field a decoder would otherwise trust to slice with.
        let mut lying = out.clone();
        lying[4] = 200;
        assert_eq!(Entry::from_slice(&lying), Err(refusal::MALFORMED));

        // A reserved word somebody used. Refused, never ignored: a bit silently
        // dropped is two readers with different beliefs about one record.
        let mut reserved = out.clone();
        reserved[42] = 1;
        assert_eq!(Entry::from_slice(&reserved), Err(refusal::UNKNOWN));

        // A namespace and an operation from a build that is not this one.
        let mut future = out.clone();
        future[6] = 9;
        assert_eq!(Entry::from_slice(&future), Err(refusal::UNKNOWN));
        let mut verb = out;
        verb[7] = 9;
        assert_eq!(Entry::from_slice(&verb), Err(refusal::UNKNOWN));
    }

    #[test]
    fn a_name_outside_the_bounds_is_refused_by_the_encoder() {
        let long = vec![b'x'; NAME_MAX + 1];
        let entry = Entry { namespace: ns::PATH, op: op::SET, hash: [0u8; 32], name: &long };
        assert_eq!(entry.encoded_bytes(), Err(refusal::MALFORMED));
        let empty = Entry { namespace: ns::PATH, op: op::SET, hash: [0u8; 32], name: b"" };
        assert_eq!(empty.encoded_bytes(), Err(refusal::MALFORMED));

        // And a namespace this build does not know, refused where it is written
        // rather than at somebody else's mount.
        let name = b"n";
        let alien = Entry { namespace: 200, op: op::SET, hash: [0u8; 32], name };
        let mut out = vec![0u8; HEAD_BYTES + 1];
        assert_eq!(alien.to_slice(&mut out), Err(refusal::UNKNOWN));
    }

    /// Zero is not a namespace and not an operation, which is what makes a
    /// zeroed block undecodable from two directions rather than one.
    #[test]
    fn zero_is_not_a_namespace_and_not_an_operation() {
        assert!(!ns::known(0));
        assert!(!op::known(0));
        assert_eq!(ns::label(0), "unknown");
        assert_eq!(ns::label(ns::PIN), "pin", "RFC 0059's namespace exists and is named");
    }
}
