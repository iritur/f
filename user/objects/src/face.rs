// SPDX-License-Identifier: Apache-2.0 OR MIT
//! A face out of the blob store, by the content address this component's own
//! manifest declares — and the refusal of one it does not.
//!
//! # What `E3-B03b` asks for, and which half of it is here
//!
//! *A face is addressed by content hash and declared before it is used*, and the
//! exit is a boot that loads one out of the store by hash and refuses one the
//! manifest did not declare. Three things have to be true for that sentence to
//! mean anything, and they are in three different crates on purpose:
//!
//! - **The declaration is in the manifest.** `user/objects/manifest.toml` has a
//!   `[[face]]` table, `cargo xtask lint-manifests` refuses one that does not
//!   fit `docs/manifest.md`, and `cargo xtask component` compiles it into a
//!   section of the component file after the image. `abi::manifest::Face` is
//!   that section and argues at length why it is not a field of the record.
//! - **The bytes are a face.** `f_text::face` decides that, and deliberately
//!   decides nothing else.
//! - **The address is the authority.** This module. Nothing here believes a
//!   name, a length or a shape: it is handed the declared entries the frame read
//!   out of the record, and the only question it answers is *is this address one
//!   of them*.
//!
//! # Why the refusal happens before the store is asked
//!
//! Because *declared before it is used* is a claim about ordering, and a check
//! taken after a `get` is a check taken after the bytes have already moved into
//! a buffer somebody else can see. [`load_at`] refuses an undeclared address
//! without touching the store at all, and
//! `an_undeclared_address_never_reaches_the_store` is the test that would go red
//! if the two were swapped — which is exactly the mistake a later refactor makes
//! when it tidies the two lookups into one.
//!
//! # Why this component stocks the face it then loads
//!
//! Because there is no device under this component — `crate::serve`'s comment
//! says so at length — so the store it reads is one in its own heap. What that
//! costs is that the face has to get into it, and what it buys is that the
//! **address is not a constant anywhere**: [`stock`] composes the bytes,
//! `f_blob` hashes them, and the manifest declares the digest of the same
//! composition. Two independent computations of one content, meeting at an
//! address neither side chose, which is the arrangement `crate::serve` already
//! uses for a read's content and the reason a passing test here is evidence.
//!
//! `the_manifest_declares_the_address_of_what_this_component_stocks` is where
//! the two meet, and it reads the manifest as text rather than as a compiled
//! record because that is the file a person edits.
//!
//! # Determinism
//!
//! No clock, no draw, no map, no float. The composition is a function of the
//! constants below and of nothing else, so its address is the same on both
//! architectures — which is the property a declared content address has to have
//! before it is worth declaring.

use f_abi::manifest;
use f_abi::store::kind;
use f_blob::device::Device;
use f_blob::store::Store;
use f_hash::sha256;
use f_text::face::{self, Face};

/// The design-unit grid of the face this component stocks.
///
/// `f_text::face::fixture`'s and not this crate's own, and the indirection is
/// the point: the frame computes the same address for itself before it decides
/// whether a component may load it, so a second transcription of these numbers
/// here would be a second face with a different address — and the symptom would
/// be a read that resolves to nothing rather than a diff anybody can see.
/// Unit: design units per em.
pub const UNITS_PER_EM: u32 = face::fixture::UNITS_PER_EM;

/// The advances of the face this component stocks, glyph by glyph.
///
/// [`face::fixture::DECLARED`], for [`UNITS_PER_EM`]'s reason. The argument for
/// *these four values* is there rather than here.
/// Unit: design units.
pub const ADVANCES: [u16; 4] = face::fixture::DECLARED;

/// The advances of the face this component stocks and **nothing declares**.
///
/// [`face::fixture::UNDECLARED`]: the same face with one advance one design unit
/// larger. It is stocked beside the declared one so that `E3-B03b`'s second
/// clause is about a face that is real, stored and readable and is refused for
/// its address alone — a store that did not hold it would make the refusal
/// ambiguous between a permission and a miss, which is the distinction
/// [`Refusal`] has two variants for.
/// Unit: design units.
pub const UNDECLARED: [u16; 4] = face::fixture::UNDECLARED;

/// How many bytes the composed face occupies, and therefore how large a buffer
/// both halves of this module need.
/// Unit: bytes.
pub const BYTES: usize = face::bytes_for(ADVANCES.len());

/// The name the manifest declares this face under, and the name a caller asks
/// by.
///
/// It is this component's own vocabulary and nothing outside the component reads
/// it; what crosses is the address. It is here rather than in the manifest alone
/// because a caller has to spell it, and it is compared against the manifest by
/// [`load`]'s own lookup rather than trusted — a name this component asks for
/// that no `[[face]]` declares is [`Refusal::Undeclared`] like any other.
pub const REGULAR: &[u8] = b"regular";

/// Why a face was not loaded.
///
/// Four, and the first one is the task. The others exist so that a boot which
/// goes red says *which* of the four things went wrong rather than *a face did
/// not come back*, which is a failure somebody debugs by bisecting a store.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Refusal {
    /// No `[[face]]` entry in this component's record names this address, or
    /// this name. **This is `E3-B03b`'s refusal** and it is taken before the
    /// store is asked anything at all.
    Undeclared,
    /// The store would not answer: a packed refusal from `f_blob`, which is an
    /// address it holds nothing under, a device that failed, or a buffer too
    /// small for what is there.
    /// Unit: none — a packed error, RFC 0010.
    NotStocked(i32),
    /// The bytes came back and are not a face.
    Malformed(face::Refusal),
}

/// Compose the face this component stocks, into `into`.
///
/// # Errors
///
/// Whatever `f_text::face::compose` refuses, which for these constants is only
/// a buffer shorter than [`BYTES`].
pub fn compose(into: &mut [u8]) -> Result<usize, face::Refusal> {
    face::fixture::declared(into)
}

/// Compose the face nothing declares, into `into`.
///
/// # Errors
///
/// As [`compose`].
pub fn compose_undeclared(into: &mut [u8]) -> Result<usize, face::Refusal> {
    face::fixture::undeclared(into)
}

/// Compose the face and put it in the store, and answer the address the store
/// gave it.
///
/// The address is **the store's answer and not an argument**: nothing here
/// compares it against the manifest, because a stocker that checked its own work
/// against the declaration would be the same code deciding both sides. The
/// comparison is [`load`]'s, at the moment it matters, and the boot's.
///
/// # Errors
///
/// [`Refusal::Malformed`] if the scratch buffer is too small for [`BYTES`], and
/// [`Refusal::NotStocked`] for whatever the store says about a write.
pub fn stock<D: Device>(store: &mut Store<D>, scratch: &mut [u8]) -> Result<[u8; 32], Refusal> {
    put(store, scratch, compose)
}

/// Compose the face nothing declares and put it in the store, and answer the
/// address the store gave it.
///
/// **A second function and not a flag**, for `crate::serve`'s reason about its
/// own provocation: a grep for this name finds every place this component puts
/// a face nobody declared into a store, and there is exactly one. A boolean
/// argument would put both faces behind one call site and make the answer *read
/// the branch*.
///
/// # Errors
///
/// As [`stock`].
pub fn stock_undeclared<D: Device>(
    store: &mut Store<D>,
    scratch: &mut [u8],
) -> Result<[u8; 32], Refusal> {
    put(store, scratch, compose_undeclared)
}

/// Compose with `writer` and put the result in the store.
///
/// The body [`stock`] and [`stock_undeclared`] share, so that what differs
/// between a declared face and an undeclared one is the bytes and nothing about
/// how they reach the store. Two stockers with two code paths would be two
/// answers to *is this face in the store the same way the other one is*.
fn put<D: Device>(
    store: &mut Store<D>,
    scratch: &mut [u8],
    writer: fn(&mut [u8]) -> Result<usize, face::Refusal>,
) -> Result<[u8; 32], Refusal> {
    let len = writer(scratch).map_err(Refusal::Malformed)?;
    let bytes = scratch.get(..len).ok_or(Refusal::Malformed(face::Refusal::Truncated))?;
    // `kind::CHUNK` and not a fifth kind: a face is a run of bytes named by
    // SHA-256 over exactly those bytes, which is that kind's definition word for
    // word, and `abi::store::kind` forecloses a fifth one as a diff to `abi/`
    // and an RFC by rule. RFC 0058.
    store.put(kind::CHUNK, bytes).map_err(Refusal::NotStocked)
}

/// Load the face at `hash`, if this component declared it.
///
/// `declared` is the face table of this component's own record — what
/// `abi::manifest::Record::faces` hands back over the module the loader placed.
/// It is a parameter and not something this module reads for itself, because a
/// component that could produce its own declaration is a component that is its
/// own authority, which is the one thing the task is about.
///
/// # Errors
///
/// [`Refusal::Undeclared`] when no entry names this address — taken **first**,
/// before the store is asked; then whatever the store or `f_text::face` says.
pub fn load_at<'a, D: Device>(
    declared: &[manifest::Face],
    store: &mut Store<D>,
    hash: &[u8; 32],
    into: &'a mut [u8],
) -> Result<Face<'a>, Refusal> {
    if !declared.iter().any(|entry| entry.hash == *hash) {
        return Err(Refusal::Undeclared);
    }
    let len = store.get(hash, into).map_err(Refusal::NotStocked)?;
    let bytes = into.get(..len).ok_or(Refusal::Malformed(face::Refusal::Truncated))?;
    Face::read(bytes).map_err(Refusal::Malformed)
}

/// Load the face this component's manifest declares under `name`.
///
/// The name is resolved against the declaration rather than against anything
/// this crate holds, so a component asking for a face it did not declare is the
/// same refusal as a component asking for an address it did not declare — which
/// is the reading that makes the manifest the single place the set is written.
///
/// # Errors
///
/// As [`load_at`], with [`Refusal::Undeclared`] also covering a name no
/// `[[face]]` entry carries.
pub fn load<'a, D: Device>(
    declared: &[manifest::Face],
    store: &mut Store<D>,
    name: &[u8],
    into: &'a mut [u8],
) -> Result<Face<'a>, Refusal> {
    let entry = declared.iter().find(|entry| entry.label() == name).ok_or(Refusal::Undeclared)?;
    let hash = entry.hash;
    load_at(declared, store, &hash, into)
}

/// The address the bytes this component composes hash to.
///
/// Computed rather than written down, so that a change to [`ADVANCES`] or
/// [`UNITS_PER_EM`] moves it and the manifest stops agreeing — which is the test
/// below going red rather than a component quietly stocking a face nobody
/// declared.
///
/// # Errors
///
/// Whatever [`compose`] refuses.
pub fn address(scratch: &mut [u8]) -> Result<[u8; 32], face::Refusal> {
    let len = compose(scratch)?;
    Ok(sha256(scratch.get(..len).ok_or(face::Refusal::Truncated)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use f_blob::device::Memory;
    use f_blob::store::superblock_for_this_build;

    /// This component's manifest, as the file a person edits.
    ///
    /// Read as text and not as a compiled record, deliberately: the thing that
    /// has to be true is that *the declaration a person wrote* names what this
    /// component stocks. A test that read the record would be a test of
    /// `cargo xtask component`, which has its own.
    const MANIFEST: &str = include_str!("../manifest.toml");

    /// Block size and count are `blob/src/store.rs`'s own unit-test numbers,
    /// because what is being tested here is not the store.
    fn store() -> Store<Memory> {
        let layout = superblock_for_this_build(512, 8, 1 << 20, 1, 2);
        Store::format(Memory::new(512, 64), &layout).expect("a device this store can format")
    }

    /// Every `hash` value in the manifest's `[[face]]` tables, in file order.
    ///
    /// A hand-rolled scan rather than a parser, because the crate that has one
    /// is `xtask` and this crate may not depend on it. It is written to fail
    /// loudly rather than quietly: a manifest whose spelling moved produces an
    /// empty list, and every caller asserts the list is not empty.
    fn declared_in_the_manifest() -> alloc::vec::Vec<[u8; 32]> {
        let mut out = alloc::vec::Vec::new();
        for line in MANIFEST.lines() {
            let line = line.trim();
            let Some(rest) = line.strip_prefix("hash") else { continue };
            let Some(rest) = rest.trim_start().strip_prefix('=') else { continue };
            let rest = rest.trim().trim_matches('"');
            let Some(hex) = rest.strip_prefix("sha256:") else { continue };
            let bytes = hex.as_bytes();
            assert_eq!(bytes.len(), 64, "a content address is sixty-four hex digits");
            let mut hash = [0u8; 32];
            for (index, byte) in hash.iter_mut().enumerate() {
                let nibble = |at: usize| -> u8 {
                    match bytes[at] {
                        c @ b'0'..=b'9' => c - b'0',
                        c @ b'a'..=b'f' => c - b'a' + 10,
                        other => panic!("`{}` is not a lower-case hex digit", other as char),
                    }
                };
                *byte = nibble(index * 2) * 16 + nibble(index * 2 + 1);
            }
            out.push(hash);
        }
        out
    }

    /// The declaration the frame would hand this module, built from the
    /// manifest's own text.
    fn declaration() -> alloc::vec::Vec<manifest::Face> {
        declared_in_the_manifest()
            .into_iter()
            .map(|hash| {
                let mut entry = manifest::Face::EMPTY;
                entry.name[..REGULAR.len()].copy_from_slice(REGULAR);
                entry.hash = hash;
                entry
            })
            .collect()
    }

    #[test]
    fn the_manifest_declares_the_address_of_what_this_component_stocks() {
        // The whole of *declared before it is used*, at the level a person can
        // edit: the digest in `user/objects/manifest.toml` is the digest of the
        // bytes this crate composes, computed here rather than copied.
        let mut scratch = [0u8; BYTES];
        let computed = address(&mut scratch).expect("the composition is one the writer admits");
        let declared = declared_in_the_manifest();
        assert_eq!(
            declared.len(),
            1,
            "`user/objects/manifest.toml` declares exactly one `[[face]]`; if that moved, so did \
             this test's reading of it"
        );
        assert_eq!(
            declared[0], computed,
            "the manifest's `[[face]] hash` is not the address of what `face::compose` produces"
        );
    }

    #[test]
    fn the_store_gives_the_face_the_address_the_manifest_declares() {
        // The second independent computation: `f_blob` hashes what it was
        // handed, and it lands on the number a person wrote in a file.
        let mut store = store();
        let mut scratch = [0u8; BYTES];
        let at = stock(&mut store, &mut scratch).expect("a face this store will hold");
        assert_eq!(at, declared_in_the_manifest()[0]);
    }

    #[test]
    fn a_declared_face_is_loaded_and_is_what_was_stocked() {
        let mut store = store();
        let mut scratch = [0u8; BYTES];
        stock(&mut store, &mut scratch).expect("stocked");
        let declared = declaration();
        let mut into = [0u8; BYTES];
        let face = load(&declared, &mut store, REGULAR, &mut into).expect("a declared face");
        assert_eq!(face.units_per_em(), UNITS_PER_EM);
        assert_eq!(face.glyphs(), ADVANCES.len());
        for (glyph, expect) in ADVANCES.iter().enumerate() {
            assert_eq!(face.advance_design_units(glyph), Some(*expect));
        }
    }

    #[test]
    fn an_undeclared_face_is_refused_although_it_is_real_stored_and_readable() {
        // The task's second clause. The other face is the same face with one
        // advance one design unit larger: it composes, it is put in the store,
        // `f_text::face` reads it back happily, and none of that is the
        // question. What decides is that no `[[face]]` entry names its address.
        let mut store = store();
        let mut scratch = [0u8; BYTES];
        stock(&mut store, &mut scratch).expect("stocked");
        let declared = declaration();

        let mut bytes = [0u8; BYTES];
        let len = compose_undeclared(&mut bytes).expect("also a face");
        let mut into_the_store = [0u8; BYTES];
        let elsewhere = stock_undeclared(&mut store, &mut into_the_store)
            .expect("a store that holds the other one too");
        assert_ne!(elsewhere, declared[0].hash, "the two faces have two addresses");

        let mut into = [0u8; BYTES];
        assert!(
            f_text::face::Face::read(&bytes[..len]).is_ok(),
            "the refusal must not be about the bytes being unreadable"
        );
        assert_eq!(load_at(&declared, &mut store, &elsewhere, &mut into), Err(Refusal::Undeclared));
    }

    #[test]
    fn an_undeclared_address_never_reaches_the_store() {
        // The ordering clause, and it is the one a later tidy-up breaks. The
        // store here holds nothing at all: if the refusal were taken after the
        // `get`, this would come back `NotStocked` instead, which is a different
        // sentence about a different failure.
        let mut store = store();
        let declared = declaration();
        let mut into = [0u8; BYTES];
        let never = [0x5au8; 32];
        assert_eq!(load_at(&declared, &mut store, &never, &mut into), Err(Refusal::Undeclared));
    }

    #[test]
    fn a_name_no_entry_carries_is_the_same_refusal() {
        let mut store = store();
        let mut scratch = [0u8; BYTES];
        stock(&mut store, &mut scratch).expect("stocked");
        let declared = declaration();
        let mut into = [0u8; BYTES];
        assert_eq!(load(&declared, &mut store, b"italic", &mut into), Err(Refusal::Undeclared));
    }

    #[test]
    fn a_declared_face_nothing_stocked_is_not_the_same_refusal_as_an_undeclared_one() {
        // Two failures that look alike from a distance and are not: one is a
        // component asking for something it may not have, the other is a store
        // that does not hold what it should. A boot that printed one word for
        // both would be a boot somebody debugs by bisecting.
        let mut store = store();
        let declared = declaration();
        let mut into = [0u8; BYTES];
        let hash = declared[0].hash;
        assert!(matches!(
            load_at(&declared, &mut store, &hash, &mut into),
            Err(Refusal::NotStocked(_))
        ));
    }
}
