// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The frame measuring itself, and deciding whether it may publish a root.
//!
//! RFC 0012 is the decision and this is one of its two implementations; the
//! other is `xtask/src/measure.rs`, which takes the same two ranges out of the
//! linked ELF on a host. The two are separate on purpose: a build side and a
//! boot side that shared code would agree with themselves and say nothing.
//!
//! # What is measured, and in which order
//!
//! `__text_start .. __text_end`, then `__rodata_start .. __rodata_end`, fed to
//! one SHA-256 state in that order. Both boundaries come from `kernel/linker.ld`
//! and both are page-aligned there already, because the mapper needs them to be.
//!
//! # When
//!
//! After the address-space switch has made those two ranges read-only and not
//! writable, and before anything is published. That ordering is load-bearing in
//! both directions. Earlier and the pages the frame hashes are still writable by
//! the code doing the hashing, so the measurement would be of memory that could
//! still change; later and something could have been published under a root the
//! frame had not yet earned the right to publish.
//!
//! # What this proves, stated narrowly because the RFC is narrow
//!
//! It catches a modified image booted honestly — the reason
//! [`crate::measure`]'s own deliberate defect exists and the reason `cargo xtask
//! attest` boots one. It does **not** catch an image modified to report the old
//! digest: a self-hash is a claim by the thing being measured, and the residual
//! closes with a measurement taken before the frame runs, which is a TPM and
//! E5's hardware. It says nothing about memory after this instant, nothing about
//! freshness, and nothing about data. RFC 0012 lists all five; this comment
//! exists so that a reader of the code meets the list at the same time as the
//! code.
//!
//! # What is outside the ranges, and why that is a decision
//!
//! `.boot` — unreachable after the address-space switch, so covering it would
//! mean hashing before the mapper exists — `.data` and `.got` initialisers,
//! `.bss`, `.stacks`, the multiboot command line including the two tokens below,
//! the boot module set, the loader and the firmware. A frame change confined to
//! a writable initial value is invisible here. RFC 0012 names that as one of its
//! own reversal conditions rather than as a footnote.

use f_abi::boot::{Declaration, Selection};
use f_hash::Sha256;

unsafe extern "C" {
    static __text_start: u8;
    static __text_end: u8;
    static __rodata_start: u8;
    static __rodata_end: u8;
}

/// A byte in the frame's read-only data whose only job is to be different in a
/// build with the defect on.
///
/// This is the modification `cargo xtask attest` makes on purpose, and its shape
/// is the argument: it changes **nothing the machine does**. Nothing reads it,
/// no branch depends on it, no log line mentions it. So the only thing in the
/// system that can notice the build changed is the measurement — which is
/// exactly the sentence the exit is about, and which a defect that also broke
/// something would not have demonstrated, because the broken thing would have
/// been what went red.
///
/// `#[used]` because nothing reads it: without that the linker is free to drop
/// the symbol and the defect would quietly build a byte-identical image. The
/// same reason `main::BUILT_AT` carries it, one defect over.
///
/// [`self_test`] requires it to land inside the measured range. An immutable
/// array of bytes has no relocations and belongs in `.rodata`, but *belongs* is
/// a fact about a compiler rather than a promise, and a mark that had drifted
/// into `.data` would make the defect invisible while looking correct.
#[used]
static MARK: [u8; 16] = if cfg!(feature = "mutate-modified-frame") { [0xA5; 16] } else { [0; 16] };

/// The frame's measurement of itself, and what it was told to expect.
#[derive(Clone, Copy, Debug)]
pub struct Identity {
    /// SHA-256 over the two ranges of this running image, in RFC 0012's order.
    /// Unit: bytes, exactly 32 of them — a digest.
    pub measured: [u8; 32],
    /// The generation root the loader selected with `f.root=`, or `None` when it
    /// selected none.
    /// Unit: bytes, exactly 32 of them — a content address.
    pub root: Option<[u8; 32]>,
    /// The frame hash the generation this boot was selected into declares, from
    /// `f.frame=`, or `None` when the command line carried no declaration.
    /// Unit: bytes, exactly 32 of them — a digest.
    pub declared: Option<[u8; 32]>,
    /// How many bytes went into [`Identity::measured`].
    /// Unit: bytes.
    pub covered: u64,
}

impl Identity {
    /// Whether this boot may publish a root, and the generation counter to
    /// publish with it.
    ///
    /// **Zero is not an error.** RFC 0012 reserves it for *no root describes this
    /// machine at this instant*, and a boot the loader gave no `f.root=` is
    /// exactly that: the frame knows what it is running and has not been told
    /// which generation that is. One is the first publish. The counter is never
    /// anything else here, because this frame publishes a root once and does not
    /// swap — `E2-B06` is the task that makes this a number rather than a
    /// predicate.
    /// Unit: count of publishes; 0 is reserved and means none.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        match self.root {
            Some(_) => 1,
            None => 0,
        }
    }

    /// Does the declaration this boot was given agree with what it measured?
    ///
    /// `true` when there is no declaration, and the asymmetry is deliberate: a
    /// boot with no `f.root=` publishes no root, so there is no root whose frame
    /// field could be wrong. What must never happen is a root published beside a
    /// frame field that disagrees with the image that published it.
    #[must_use]
    pub fn agrees(&self) -> bool {
        match self.declared {
            Some(declared) => declared == self.measured,
            None => true,
        }
    }
}

/// Why the frame refused to publish a root.
#[derive(Clone, Copy, Debug)]
pub enum Refusal {
    /// The command line named a generation root and no frame hash, or the
    /// reverse. The two are one statement — *this is the generation, and this is
    /// the frame it was compiled against* — and half of it is not a weaker
    /// statement but a different one.
    HalfADeclaration,
    /// A token this build could not parse.
    Malformed,
    /// The measurement and the declaration disagree.
    ///
    /// RFC 0012: *disagreement is a refusal to publish a root, not a warning
    /// line.* Either the image was modified after the generation was compiled,
    /// or the machine was pointed at a generation compiled against a different
    /// frame. The frame cannot tell those apart and does not guess.
    FrameDisagrees,
    /// The deliberate-defect mark is outside the measured range, so a
    /// modification to it would not move the digest.
    MarkOutside,
}

impl Refusal {
    /// A line for the boot log.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::HalfADeclaration => {
                "the command line names a root without a frame hash, or the reverse"
            }
            Self::Malformed => "a boot token this build cannot parse",
            Self::FrameDisagrees => {
                "the frame this image measures is not the frame the generation declares"
            }
            Self::MarkOutside => "the measured range does not contain the mark it must cover",
        }
    }
}

/// Hash this image's text and rodata, in RFC 0012's order.
///
/// # Panics
///
/// Never: both ranges are exported by the linker script with `end` above
/// `start`, and a range whose ends had been swapped would produce a length of
/// zero here rather than a wild slice.
#[must_use]
pub fn frame() -> ([u8; 32], u64) {
    let mut state = Sha256::new();
    let mut covered = 0u64;
    for (start, end) in ranges() {
        // SAFETY: `start` and `end` are linker-exported boundaries of an output
        // section of this image, mapped and read-only at this point in the boot,
        // and `len` is their difference, so the slice is entirely inside one
        // live mapping. Saturating rather than wrapping: a linker script whose
        // `end` fell below its `start` would produce an empty range and a digest
        // the build side would refuse, not a slice over the whole address space.
        let len = end.saturating_sub(start) as usize;
        // SAFETY: as above. `u8` has no alignment requirement and no invalid bit
        // pattern, and nothing writes these pages while this runs — the mapper
        // made them read-only before this is called, which is the ordering the
        // module comment states.
        let body = unsafe { core::slice::from_raw_parts(start as *const u8, len) };
        state.update(body);
        covered = covered.saturating_add(len as u64);
    }
    (state.finish(), covered)
}

/// The two ranges, in the order they are hashed.
///
/// A function rather than four call sites reading four externs, because the
/// *order* is the wire: text then rodata produces a different digest from rodata
/// then text over the same bytes, and one place that states the order is one
/// place a reader has to check it against `xtask/src/measure.rs`'s.
fn ranges() -> [(u64, u64); 2] {
    [
        ((&raw const __text_start) as u64, (&raw const __text_end) as u64),
        ((&raw const __rodata_start) as u64, (&raw const __rodata_end) as u64),
    ]
}

/// Measure, and read what the command line says this machine is.
///
/// **A disagreement is not an error here**, and that is the one design decision
/// in this function. It would be simpler to refuse inside and hand the caller a
/// [`Refusal::FrameDisagrees`], and the boot log would then say *the frame this
/// image measures is not the frame the generation declares* without ever saying
/// **what it measured** — which is the first thing anybody debugging that line
/// wants and the only evidence that the refusal was a disagreement about a
/// number rather than a token that failed to parse. So the comparison is
/// [`Identity::agrees`], the caller prints before it decides, and the refusal is
/// the caller's. `cargo xtask attest` reads both lines out of that log.
///
/// # Errors
///
/// A [`Refusal`] for the two things that are not a disagreement: a token this
/// build cannot parse, and half a declaration. Both mean the command line is not
/// a statement about this machine at all, so there is nothing to print.
pub fn identity(cmdline: &[u8]) -> Result<Identity, Refusal> {
    self_test()?;
    let (measured, covered) = frame();

    let mut root = None;
    let mut declared = None;
    for word in cmdline.split(|byte| *byte == b' ') {
        if word.starts_with(f_abi::boot::KEY.as_bytes()) {
            root = Some(Selection::parse(word).map_err(|_| Refusal::Malformed)?.root);
        } else if word.starts_with(f_abi::boot::FRAME_KEY.as_bytes()) {
            declared = Some(Declaration::parse(word).map_err(|_| Refusal::Malformed)?.frame);
        }
    }
    if root.is_some() != declared.is_some() {
        return Err(Refusal::HalfADeclaration);
    }

    Ok(Identity { measured, root, declared, covered })
}

/// The mark is inside the range that is measured.
///
/// A check that the demonstration is possible rather than a check on the
/// measurement itself, and it runs on every boot rather than on the defect's:
/// the failure it catches is the *clean* build putting the mark somewhere the
/// digest does not cover, which would make `cargo xtask attest`'s modified boot
/// publish the same hash and look like the property failing when what failed was
/// the provocation. The same argument every provoked counter in
/// `kernel/src/state.rs` makes.
fn self_test() -> Result<(), Refusal> {
    let at = (&raw const MARK) as u64;
    let end = at.saturating_add(core::mem::size_of_val(&MARK) as u64);
    let (start, stop) = ranges()[1];
    if at >= start && end <= stop { Ok(()) } else { Err(Refusal::MarkOutside) }
}

/// A digest, ready to be printed as the sixty-four lower-case characters this
/// tree spells one with.
///
/// A type rather than a formatting call at each site, because the boot log is
/// the artefact `cargo xtask trace` hashes and `cargo xtask attest` parses: two
/// call sites that spelled a hash differently would be two lines a reader has to
/// compare by eye and a parser has to have two cases for.
///
/// It is deliberately *not* `f_abi::boot`'s renderer with the key trimmed off. A
/// log line is not a token, and a log that printed `f.root=<hex>` would invite a
/// reader to paste one into a command line and get a selection that happened to
/// be right.
pub struct Hex(pub [u8; 64]);

impl core::fmt::Display for Hex {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for byte in self.0 {
            f.write_str(match core::str::from_utf8(core::slice::from_ref(&byte)) {
                Ok(text) => text,
                // Unreachable: every byte written below is an ASCII digit. A
                // question mark rather than an error, because a boot log line
                // that could fail to print would be a boot that could fail on
                // its own diagnostics.
                Err(_) => "?",
            })?;
        }
        Ok(())
    }
}

/// Render thirty-two bytes as sixty-four lower-case hexadecimal characters.
#[must_use]
pub fn hex(digest: &[u8; 32]) -> Hex {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = [0u8; 64];
    for (n, byte) in digest.iter().enumerate() {
        out[2 * n] = DIGITS[(byte >> 4) as usize];
        out[2 * n + 1] = DIGITS[(byte & 0xF) as usize];
    }
    Hex(out)
}
