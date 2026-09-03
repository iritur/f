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

/// What a trace that begins part-way through a run knows about the part before
/// it.
///
/// # Why this exists at all
///
/// `E1-P08` measured the obvious design and found it did not pay. A snapshot
/// that carries the whole artefact restores a run that is indistinguishable from
/// the run that replayed — the oracle included — but reading a record back costs
/// the same *order* as taking a step, so re-entering a run near its end costs a
/// large fraction of replaying it. The measurement is in RFC 0043 and the ratio
/// was under two.
///
/// The saving comes from the artefact **not** travelling, and what it costs is
/// exactly the part of the artefact that is not there. So a terse snapshot
/// carries this instead: the running hash of the prefix, and the counts. The
/// restored run then produces the same [`Trace::digest`] as the whole run — the
/// number every reproduction check in this tree compares — and the same records
/// from the cut onward, and it cannot be judged by `check`, which reads the
/// whole artefact. `check::examine` refuses one rather than judging it, which is
/// R04 at the one place a fast path could quietly answer a different question.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Carried {
    /// FNV-1a of the artefact up to the cut. Unit: none.
    pub hash: u64,
    /// How many bytes were folded into it. Unit: bytes.
    ///
    /// Needed because the artefact joins its lines with a newline: a tail folded
    /// onto a non-empty prefix has to fold that newline first, and a tail folded
    /// onto an empty one must not.
    pub bytes: u64,
    /// Records before the cut. Unit: records.
    pub records: u64,
    /// Header lines before it. Unit: lines.
    pub covers: u64,
    /// Where the clock stood at the cut. Unit: nanoseconds.
    pub at_ns: u64,
}

/// Everything a run wrote down, in order, under a header saying what it covers.
///
/// # The running fold, and why it is a cache rather than a field
///
/// [`Trace::digest`] is FNV-1a over [`Trace::text`], and building that text over
/// half a million records costs about as much as the run that produced them.
/// One digest at the end of a run is fine; `E1-P08` asks for one at every mark
/// of a scan, and forty of those turned a one-second run into a sixteen-second
/// one.
///
/// So the fold is remembered, and only the records since the last one are
/// folded. It is a *cache* and not an eagerly-maintained field because most runs
/// never ask for a digest at all — `cargo xtask sweep` runs a million trials
/// through the oracle and hashes none of them — and folding on every `push`
/// would put the cost on the path that does not want it.
///
/// `Cell` rather than `&mut self`, because `Outcome::digest` takes `&self` and
/// so does every caller of it. The cache is not part of what a trace *is*, which
/// is why [`PartialEq`] below is written out rather than derived.
#[derive(Clone, Debug)]
pub struct Trace {
    cover: Vec<String>,
    records: Vec<Record>,
    /// The part of the artefact this trace does not hold, when it begins
    /// part-way through a run. `None` for every trace a run starts with.
    carried: Option<Carried>,
    /// The fold so far: the hash, the bytes folded into it, and how many header
    /// lines and records that covers.
    running: core::cell::Cell<(u64, u64, usize, usize)>,
}

impl Default for Trace {
    fn default() -> Self {
        Self {
            cover: Vec::new(),
            records: Vec::new(),
            carried: None,
            running: core::cell::Cell::new((BASIS, 0, 0, 0)),
        }
    }
}

impl PartialEq for Trace {
    /// Two traces are equal when they say the same thing.
    ///
    /// The running fold is deliberately not compared: it is a memo of what the
    /// three fields above already determine, and a trace that had been hashed
    /// would otherwise differ from an identical one that had not.
    fn eq(&self, other: &Self) -> bool {
        self.cover == other.cover && self.records == other.records && self.carried == other.carried
    }
}

impl Eq for Trace {}

/// The FNV-1a offset basis, which is where an unfolded artefact starts.
const BASIS: u64 = 0xcbf2_9ce4_8422_2325;

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

    /// What this trace does not hold, when it begins part-way through a run.
    ///
    /// `None` is a whole artefact. `Some` is a trace restored from a terse
    /// snapshot: [`Trace::digest`] is still the whole run's, [`Trace::text`] is
    /// the part from the cut onward, and `check::examine` refuses to judge it.
    #[must_use]
    pub const fn carried(&self) -> Option<Carried> {
        self.carried
    }

    /// The one number two runs are compared by.
    ///
    /// **The whole run's, even when this trace holds only part of it.** A
    /// carried prefix is a hash of the bytes that came before, and FNV-1a is a
    /// fold — so continuing it over the tail gives the number the whole artefact
    /// hashes to. That is what makes a terse re-entry comparable with a full
    /// replay at all, and it is the property
    /// `snap::tests::a_restored_run_is_the_run_that_replayed` holds over both
    /// kinds of snapshot.
    #[must_use]
    pub fn digest(&self) -> u64 {
        self.advance().0
    }

    /// Where this trace's own fold starts: the prefix it carries, or nothing.
    const fn origin(&self) -> (u64, u64) {
        match self.carried {
            None => (BASIS, 0),
            Some(carried) => (carried.hash, carried.bytes),
        }
    }

    /// Fold every line not yet folded, and answer the hash and the bytes.
    ///
    /// Linear in what has arrived since the last call, which is what makes a
    /// scan that hashes at every mark cost one fold of the run rather than one
    /// per mark.
    fn advance(&self) -> (u64, u64) {
        let (mut hash, mut bytes, mut covers, mut records) = self.running.get();
        if records > 0 && covers < self.cover.len() {
            // A header line arrived after a record. Nothing in this crate does
            // it — `Scenario::cover` writes the whole header before an actor is
            // installed — and if something starts to, the fold has to begin
            // again rather than quietly hash the lines in the wrong order.
            (hash, bytes) = self.origin();
            covers = 0;
            records = 0;
        }
        for line in self.cover.iter().skip(covers) {
            let join = bytes > 0;
            hash = fold(hash, line, join);
            bytes = bytes
                .saturating_add(u64::from(join))
                .saturating_add(u64::try_from(line.len()).unwrap_or(u64::MAX));
        }
        for record in self.records.iter().skip(records) {
            let line = record.line();
            let join = bytes > 0;
            hash = fold(hash, &line, join);
            bytes = bytes
                .saturating_add(u64::from(join))
                .saturating_add(u64::try_from(line.len()).unwrap_or(u64::MAX));
        }
        self.running.set((hash, bytes, self.cover.len(), self.records.len()));
        (hash, bytes)
    }

    /// This trace, and everything before it, as one [`Carried`].
    fn folded(&self, at_ns: u64) -> Carried {
        let (hash, bytes) = self.advance();
        let held = u64::try_from(self.records.len()).unwrap_or(u64::MAX);
        let covers = u64::try_from(self.cover.len()).unwrap_or(u64::MAX);
        let before = self.carried.unwrap_or(Carried {
            hash: BASIS,
            bytes: 0,
            records: 0,
            covers: 0,
            at_ns: 0,
        });
        Carried {
            hash,
            bytes,
            records: before.records.saturating_add(held),
            covers: before.covers.saturating_add(covers),
            at_ns,
        }
    }

    /// Write the artefact so far into a snapshot.
    ///
    /// **The whole of it, header and records, and not a rolling hash of the
    /// prefix.** A rolling hash would be smaller and would give the restored run
    /// the same digest, and it was the first design; what it cannot give is the
    /// *artefact*. `check.rs`'s oracle reads records — which tokens were issued,
    /// which clients finished — so a restored run holding only its tail would
    /// have a client that registered before the cut look like a client that
    /// never registered, and every property would fire on a run that was fine.
    /// A snapshot whose restored run fails a check the whole run passes is
    /// exactly the plausible-and-wrong answer this module exists to refuse.
    ///
    /// The cost is stated rather than hidden: a snapshot is linear in the length
    /// of the run so far, at the fixed width below per record. RFC 0043 names
    /// the number and what would reverse the choice.
    pub(crate) fn save(&self, out: &mut crate::snap::Writer, terse: bool, at_ns: u64) {
        if terse {
            // The prefix as one hash and four counts, and no records at all.
            // What this buys and what it costs is [`Carried`]'s subject.
            let carried = self.folded(at_ns);
            out.bool(true);
            out.u64(carried.hash);
            out.u64(carried.bytes);
            out.u64(carried.records);
            out.u64(carried.covers);
            out.u64(carried.at_ns);
            out.count(0);
            out.count(0);
            return;
        }
        out.bool(self.carried.is_some());
        let carried =
            self.carried.unwrap_or(Carried { hash: 0, bytes: 0, records: 0, covers: 0, at_ns: 0 });
        out.u64(carried.hash);
        out.u64(carried.bytes);
        out.u64(carried.records);
        out.u64(carried.covers);
        out.u64(carried.at_ns);
        out.count(self.cover.len());
        for line in &self.cover {
            out.str(line);
        }
        out.count(self.records.len());
        for record in &self.records {
            out.u64(record.at_ns);
            out.u32(record.who);
            out.label(record.actor);
            out.label(record.kind);
            out.u64(record.token);
            out.u64(record.detail);
        }
    }

    /// Read one back.
    ///
    /// The header lines come back as the strings they were, which is why they
    /// are written as strings and not as labels: a cover line is prose about one
    /// run — a component's name and content hash, a fault plan — and interning
    /// it would mean a table that grew with every deployment.
    pub(crate) fn load(input: &mut crate::snap::Reader<'_>) -> Self {
        let has_carried = input.bool();
        let carried = Carried {
            hash: input.u64(),
            bytes: input.u64(),
            records: input.u64(),
            covers: input.u64(),
            at_ns: input.u64(),
        };
        let carried = has_carried.then_some(carried);
        let covers = input.count(4, "more header lines than the file could hold");
        let mut cover = Vec::with_capacity(covers);
        for _ in 0..covers {
            cover.push(input.str());
        }
        let count = input.count(32, "more records than the file could hold");
        let mut records = Vec::with_capacity(count);
        for _ in 0..count {
            records.push(Record {
                at_ns: input.u64(),
                who: input.u32(),
                actor: input.label(),
                kind: input.label(),
                token: input.u64(),
                detail: input.u64(),
            });
        }
        let origin = carried.map_or((BASIS, 0), |c| (c.hash, c.bytes));
        Self { cover, records, carried, running: core::cell::Cell::new((origin.0, origin.1, 0, 0)) }
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
    fold(BASIS, text, false)
}

/// The same fold, continued from a hash somebody else started.
///
/// The one thing FNV-1a is good for here beyond being stable: it is a *fold*, so
/// a prefix hashed yesterday and a tail hashed today give the number the whole
/// would have given. [`Trace::digest`] is what needs it and [`Carried`] is why.
///
/// `join` folds the newline the artefact separates its lines with, before
/// `text`. It is the caller's to say, because whether there is a line before
/// this one is a fact about the artefact and not about the bytes.
#[must_use]
fn fold(mut hash: u64, text: &str, join: bool) -> u64 {
    let mut feed = |byte: u8| {
        if byte != b'\r' {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    if join {
        feed(b'\n');
    }
    for byte in text.as_bytes() {
        feed(*byte);
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
    fn the_running_fold_is_the_digest_of_the_text_at_every_length() {
        // The invariant the cache rests on, checked at *every* prefix rather
        // than at the end: a fold that was right after five hundred records and
        // wrong after five hundred and one would pass a test that only looked at
        // the finished artefact, and every digest in this tree would be wrong by
        // one record.
        let mut trace = Trace::new();
        trace.cover("first");
        trace.cover("second");
        assert_eq!(trace.digest(), digest(&trace.text()));
        for token in 0..200u64 {
            trace.push(Record {
                at_ns: token * 7,
                who: (token % 3) as u32,
                actor: "app",
                kind: "issue",
                token,
                detail: token * 11,
            });
            // Asked every time, so the cache is advanced by one record between
            // two of these — which is the state a scan leaves it in.
            assert_eq!(trace.digest(), digest(&trace.text()), "the fold diverged at {token}");
        }
        // And asked once more after several pushes with nothing in between,
        // which is the other order the cache can be advanced in.
        for token in 200..260u64 {
            trace.push(Record {
                at_ns: token,
                who: 0,
                actor: "app",
                kind: "done",
                token,
                detail: 0,
            });
        }
        assert_eq!(trace.digest(), digest(&trace.text()));
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
