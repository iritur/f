// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The artefact: what a run leaves behind, and the one number two runs are
//! compared by.
//!
//! # The same vocabulary as the boot's reproduction check
//!
//! `E0-P02` already answers the question *did two runs of this commit do the
//! same thing* for a QEMU boot: `cargo xtask trace --hash` prints one hash of
//! the boot log, and the CI job runs that command on two runners and compares
//! the lines. This is the same question about a simulated run, and it is
//! deliberately the same answer — same hash function, same eighteen-character
//! `{:#018x}` line, same *two commands and a comparison* shape. A second
//! vocabulary for one idea is how a project ends up with two reproduction
//! stories and no way to say which one a failure belongs to.
//!
//! [`digest`] is therefore a second copy of `xtask`'s `trace_hash`, and the
//! copy is on purpose. `xtask` hashes a boot log and must not need the simulator
//! to do it; the simulator hashes its own trace and must not need `xtask`. What
//! keeps them one function is a fixture — [`tests::the_digest_is_the_one_xtask_hashes_boot_logs_with`]
//! here and its twin in `xtask` — both hashing one string to one stated value.
//!
//! # The artefact says what it covers, and never what it was asked for
//!
//! A trace opens with a short header — `# ` lines, hashed with the rest — that
//! states what the run modelled and what it did not. RFC 0032 decided that this
//! simulator runs the system *above* the frame, and an artefact that did not say
//! so is one somebody quotes later as covering more than it does. So the
//! coverage is written into the bytes rather than into a document beside them,
//! and for the deployment scenario the header also names each component and the
//! content hash a spawn names it by — which is the seam `E1-P01` closes, because
//! the boot log prints those same hashes.
//!
//! What is deliberately **not** in the header is the seed. It is the other half
//! of the `(seed, commit)` pair, and putting it here would be the easiest
//! available way to make this apparatus worthless: the digest would move
//! whenever the seed moved, whether or not the run did, and the negative control
//! that demands a different seed give a different answer would pass without the
//! simulator having taken one different decision. A seed's evidence is the run
//! it produced and nothing else.
//!
//! # Why the record is structured and the artefact is text
//!
//! A hash over a `Debug` rendering is a hash over whatever the standard library
//! prints today. So a record is a struct with named fields and fixed-width
//! formatting, every number in decimal or hexadecimal with a stated width, and
//! nothing in the artefact that a library is free to change. The structure is
//! also what lets a test assert a property of the run — that the clock never
//! goes backwards, say — rather than grepping its own output.

/// One thing that happened, as a run writes it down.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Record {
    /// When. Unit: nanoseconds on the simulator's clock, whose zero is the
    /// start of the run.
    pub at_ns: u64,
    /// Which actor. Unit: none — an actor index, zero-based, in the order the
    /// scenario installed them.
    pub who: u32,
    /// What kind of actor it is. Unit: none — a stable label.
    pub actor: &'static str,
    /// What happened. Unit: none — a stable label.
    pub kind: &'static str,
    /// Which operation it concerns. Unit: none — an opaque token minted by the
    /// client that issued the operation. Zero is a legitimate token.
    pub token: u64,
    /// Whatever the kind says this holds. Unit: per-kind, and the actor that
    /// writes a kind is what states it.
    pub detail: u64,
}

impl Record {
    /// This record, as the line that goes into the artefact.
    ///
    /// Fixed widths throughout, and each one wide enough for the whole of its
    /// field's type rather than for the values a scenario happens to produce: a
    /// clock of twenty decimal digits, an actor index of eight hexadecimal ones,
    /// and two labels bounded by [`crate::LABEL_WIDTH`]. A column that could move
    /// is a column that makes two otherwise identical runs disagree, and the
    /// value that moves it is by definition the one nobody tested with.
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "{:020} {:08x} {:<8} {:<8} {:016x} {:016x}",
            self.at_ns, self.who, self.actor, self.kind, self.token, self.detail
        )
    }
}

/// Everything a run wrote down, in order, under a header saying what it covers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Trace {
    cover: Vec<String>,
    records: Vec<Record>,
}

impl Trace {
    /// An empty trace, covering nothing and saying so by holding no header.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add one header line, stating something this run covers.
    ///
    /// Written by the scenario before the run starts, so the header states what
    /// was set up rather than summarising afterwards what happened. The `# `
    /// prefix is added here rather than by the caller, so that a header line
    /// cannot be mistaken for a record by anything reading the artefact and a
    /// caller cannot forget it.
    pub fn cover(&mut self, line: &str) {
        self.cover.push(format!("# {line}"));
    }

    /// The header, as it appears in the artefact.
    #[must_use]
    pub fn covers(&self) -> &[String] {
        &self.cover
    }

    /// Add a record.
    pub fn push(&mut self, record: Record) {
        self.records.push(record);
    }

    /// Every record, in order.
    #[must_use]
    pub fn records(&self) -> &[Record] {
        &self.records
    }

    /// How many records. Unit: records.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Did anything happen at all?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// The artefact: the header, then one line per record, newline-separated,
    /// no trailing newline.
    #[must_use]
    pub fn text(&self) -> String {
        let mut out = String::new();
        for line in &self.cover {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(line);
        }
        for record in &self.records {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&record.line());
        }
        out
    }

    /// The one number two runs are compared by.
    #[must_use]
    pub fn digest(&self) -> u64 {
        digest(&self.text())
    }
}

/// A stable hash of a trace.
///
/// FNV-1a, for the reasons `xtask`'s `trace_hash` states and which are repeated
/// here because this is the copy a reader of this crate will find: what it has
/// to be is *identical on two machines at one commit*, which rules out anything
/// the standard library reserves the right to change and anything seeded per
/// process — `DefaultHasher` is both. It does not have to be
/// collision-resistant, because nothing adversarial produces these traces; a
/// content-addressed release is a different problem with a different answer.
///
/// Carriage returns are skipped, exactly as `xtask` skips them in a serial log.
/// Nothing here emits one, and the line stays because the two functions being
/// the same function is the property, not the two functions being minimal.
#[must_use]
pub fn digest(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        if *byte == b'\r' {
            continue;
        }
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The string both hash functions are pinned against, and the value they
    /// must both produce. Short, printable, and containing a carriage return —
    /// which is the one behaviour of this function that is not obvious from its
    /// four lines.
    const FIXTURE: &str = "F reproduction fixture\r\n0123456789";

    /// FNV-1a of [`FIXTURE`], with the carriage return skipped.
    const FIXTURE_DIGEST: u64 = 0xea6c_1d51_99fa_61cd;

    #[test]
    fn the_digest_is_the_one_xtask_hashes_boot_logs_with() {
        // The fixture that keeps two copies of one function one function. Its
        // twin is `the_sim_digest_is_the_one_this_file_hashes_boot_logs_with` in
        // `xtask/src/main.rs`, over the same string, against the same constant.
        // If this value ever has to change, both change or the two reproduction
        // checks have quietly stopped speaking one language.
        assert_eq!(digest(FIXTURE), FIXTURE_DIGEST);
    }

    #[test]
    fn a_carriage_return_does_not_reach_the_digest() {
        assert_eq!(digest("ab\r\ncd"), digest("ab\ncd"));
        assert_ne!(digest("ab\ncd"), digest("abcd"));
    }

    #[test]
    fn a_record_has_the_same_width_whatever_it_holds() {
        let small =
            Record { at_ns: 0, who: 0, actor: "client", kind: "issue", token: 0, detail: 0 };
        let large = Record {
            at_ns: u64::MAX,
            who: 99,
            actor: "service",
            kind: "complete",
            token: u64::MAX,
            detail: u64::MAX,
        };
        assert_eq!(small.line().len(), large.line().len(), "a column moved");
    }

    #[test]
    fn the_header_is_part_of_the_artefact() {
        // The header is hashed with everything else, because a coverage
        // statement that could be edited without moving the digest is a
        // coverage statement that is decoration.
        let mut bare = Trace::new();
        bare.push(Record { at_ns: 0, who: 0, actor: "app", kind: "issue", token: 0, detail: 0 });
        let mut covered = bare.clone();
        covered.cover("covers the components and not the frame");
        assert_ne!(bare.digest(), covered.digest(), "a coverage line changed nothing");
        assert!(covered.text().starts_with("# covers"), "the header comes first");
        assert_eq!(covered.covers().len(), 1);
    }

    #[test]
    fn the_artefact_is_one_line_per_record() {
        let mut trace = Trace::new();
        assert!(trace.is_empty());
        assert_eq!(trace.text(), "");
        for at_ns in 0..3u64 {
            trace.push(Record {
                at_ns,
                who: 1,
                actor: "client",
                kind: "issue",
                token: at_ns,
                detail: 0,
            });
        }
        assert_eq!(trace.len(), 3);
        assert_eq!(trace.text().lines().count(), 3);
        assert!(!trace.text().ends_with('\n'), "a trailing newline is a byte of artefact");
    }

    #[test]
    fn two_different_traces_do_not_share_a_digest() {
        let one = |token| {
            let mut trace = Trace::new();
            trace.push(Record {
                at_ns: 1,
                who: 0,
                actor: "client",
                kind: "issue",
                token,
                detail: 0,
            });
            trace.digest()
        };
        assert_eq!(one(7), one(7));
        assert_ne!(one(7), one(8));
    }
}
