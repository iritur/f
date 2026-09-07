// SPDX-License-Identifier: Apache-2.0 OR MIT
//! What "byte-identical topology" is taken over, written as bytes rather than
//! as a promise.
//!
//! # Why this file exists at all
//!
//! `E2-B05`'s exit says *the same root produces a byte-identical topology*. A
//! test that compared two boot modules would satisfy the words and prove
//! nothing: the module is the **input**, two copies of one input are equal by
//! construction, and the comparison would be a hash comparison wearing an
//! assembler's clothes. So the thing compared has to be the assembler's
//! **output** — every decision it made — and that output has to have a byte
//! form for *byte-identical* to mean anything.
//!
//! This module is that byte form. It is not a wire format, nothing decodes it,
//! and it deliberately has no home in `abi/`: it is a statement of what the
//! topology *is*, for a comparison and for a digest, and the day something
//! parses it is the day it belongs beside the formats that are parsed.
//!
//! # What it covers, and what it deliberately does not
//!
//! Every decision that could differ between two instantiations of one root:
//!
//! - the root itself, so a rendering names what it was made from;
//! - each member, **in canonical order**: its index, its name, the content
//!   address recomputed over its file, and the five fields of its record that
//!   the topology acts on — domain, restart policy, reservation class,
//!   account, transfer mode. Not the whole record: the record's own content
//!   address is already here, and re-rendering two thousand bytes that a hash
//!   already covers would be a second statement of one fact;
//! - what it **declared** it would bind, and what it was **bound to** — the
//!   half a discovery order would corrupt if it ever reached here;
//! - each route, in canonical order: consumer, capability, source;
//! - the start order, and each member's final state.
//!
//! What it does not cover is anything with a machine in it. There is no bus
//! count, no scan order and no timing, because those are exactly the things two
//! runs may legitimately differ in while running one generation.
//!
//! # Why fixed-width and not text
//!
//! Because a text rendering compares two *formatters* as much as two
//! topologies: a field width, a separator or a decimal-versus-hex choice
//! changing would look like a topology changing. Every field below is
//! little-endian and fixed-width, so the only thing that can move a byte is a
//! decision moving.

use alloc::vec::Vec;

use f_abi::manifest::NAME_MAX;

use crate::start::State;
use crate::topology::Assembly;

/// The eight bytes a rendering begins with, so that two blobs from two builds
/// of this crate cannot be silently compared across a change to this file.
/// `F_TOP`, and an ordinal in the low half.
/// Unit: none — a fixed byte pattern.
pub const MAGIC: u64 = 0x465f_544f_5000_0001;

/// Render an instantiated topology to the bytes the exit compares.
///
/// Deterministic by construction rather than by care: every loop below walks a
/// canonical order that was fixed at instantiation, and there is no collection
/// here whose iteration order is anybody's choice.
#[must_use]
pub fn topology(assembly: &Assembly) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC.to_le_bytes());
    out.extend_from_slice(&assembly.root());
    out.extend_from_slice(&(assembly.members().len() as u32).to_le_bytes());
    out.extend_from_slice(&(assembly.routes().len() as u32).to_le_bytes());

    for member in assembly.members() {
        out.extend_from_slice(&member.index.to_le_bytes());
        out.extend_from_slice(&member.name);
        out.extend_from_slice(&member.content);

        let record = &member.record;
        out.push(record.domain);
        out.push(record.restart);
        out.push(record.class);
        out.push(record.transfer.mode);
        out.extend_from_slice(&record.memory_bytes.to_le_bytes());

        // What it declared it would drive, and what it was actually given. Both
        // halves, because the exit's whole worry is that the second one is a
        // function of the first and of a *set* of devices — never of the order
        // a scan produced them in.
        out.push(record.devices);
        for binding in record.bindings() {
            out.extend_from_slice(&binding.vendor.to_le_bytes());
            out.extend_from_slice(&binding.device.to_le_bytes());
        }
        match member.bound {
            // One is the tag for "bound", zero for "not", and a bound address
            // is three bytes after it. A sentinel address would be an address
            // that is also a legal one, which is the bug `NO_CAPABILITY` exists
            // one crate over to not have.
            Some(at) => out.extend_from_slice(&[1, at.bus, at.device, at.function]),
            None => out.extend_from_slice(&[0, 0, 0, 0]),
        }

        out.push(member.state.wire());
        // The payload a state carries, zero-extended so that every member
        // renders to the same width whatever its state: `Failed` carries a
        // packed error and `Unstarted` a member index, and a rendering whose
        // length depended on an outcome would make two topologies differ in
        // length before they differed in content.
        let payload: i64 = match member.state {
            State::Failed(why) => i64::from(why),
            State::Unstarted(source) => i64::from(source),
            State::Untried | State::Started | State::NoDevice => 0,
        };
        out.extend_from_slice(&payload.to_le_bytes());
    }

    for route in assembly.routes() {
        out.extend_from_slice(&route.component.to_le_bytes());
        out.extend_from_slice(&route.source.to_le_bytes());
        out.extend_from_slice(&route.capability);
    }

    for index in assembly.order() {
        out.extend_from_slice(&index.to_le_bytes());
    }
    out
}

/// The digest of a rendering: one line a log can carry and two runs can be
/// compared by eye.
///
/// The bytes are what the exit compares; this is what it *prints*, because
/// "two blobs were equal" is not something a reader of a log can check and
/// sixty-four characters is.
#[must_use]
pub fn digest(assembly: &Assembly) -> [u8; 32] {
    f_hash::sha256(&topology(assembly))
}

// The name field's width is part of this rendering, so a change to it is a
// change to every blob two runs compare. Pinned here rather than assumed, for
// `f_generation::fold`'s reason beside its own head width.
const _: () = assert!(NAME_MAX == 32);
