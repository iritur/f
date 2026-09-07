// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `cargo xtask rollback`: break a generation on purpose, boot the previous one
//! by name, and check that what came back is the same bytes.
//!
//! # What "the boot menu" is, because there is not one
//!
//! `E2-P07`'s exit says *roll back from the boot menu*, a reviewer found that
//! this system has no boot menu, and `intent/0006-state/spec.md` settled it
//! rather than inventing one: the menu is **the loader's own**. The kernel is
//! loaded by multiboot 1 — from QEMU's `-kernel` here and from a GRUB entry on
//! metal — so selection is a **command-line token**, `f.root=<64 hex>`, whose
//! grammar is `abi/src/boot.rs` and whose reader is `kernel/src/generation.rs`.
//! Under QEMU the menu is `-append` and a list of `-initrd` modules; on hardware
//! it is the `menuentry` block `cargo xtask generation --install` writes.
//! Nothing is imported, and in particular no GRUB source: GRUB is GPLv3 and the
//! licence boundary is not crossed by a demonstration.
//!
//! So "the menu" here is literal and is checked as one: every boot below is
//! offered **both** generations as modules, and the only thing that decides
//! which one the machine says it is, is the token.
//!
//! # The clause this command is strict about
//!
//! *Verify the restored generation is bit-identical to what it was.* A root
//! that matches is **necessary and is not that**. The fold covers the record
//! tree — each leaf's name and the content address it names — and covers none
//! of the component files that travel in the same module. Put a correct tree
//! beside a file that is not the one its leaf names and the module folds to the
//! same root; a rollback that compared roots would call it the generation it
//! rolled back to, and it is not.
//!
//! Three comparisons are therefore made and each is named where it is made:
//!
//! 1. **the root** the machine folded, against the root packed before the break
//!    — necessary, and the one a hash comparison already gives;
//! 2. **the module digest**, a SHA-256 over every byte the loader delivered:
//!    head, length array, record tree and every component file. This is the
//!    bit-identity clause and it is taken *on the machine*, over the bytes that
//!    actually arrived, not over the file that was meant to;
//! 3. **the generation rebuilt from source after the break** — same root, same
//!    module bytes, same frame leaf, same component leaves — which is what
//!    makes this a rollback rather than a re-selection of a file nobody
//!    disturbed.
//!
//! And [`tampered`] is why comparison 2 exists rather than being asserted to
//! exist: it builds the module that passes 1 and fails 2, on the host and then
//! on the machine. A check that has only ever passed is a check nobody knows
//! can fail.

use std::path::Path;

use crate::generation::{self, Packed};
use crate::pack::hex;

/// The defect that breaks the generation.
///
/// `cargo xtask mutate`'s own, reused rather than invented: it removes a bounds
/// check in the capability table, so a machine built with it dies on
/// `cap=forge` and is green on everything else. Reusing it is the point — the
/// break has to be a *real* defect in the frame, argued for somewhere, and not
/// a byte this command flipped to have something to roll back from. It also
/// lands in exactly one leaf, which is what lets step two assert that the frame
/// is the only thing that moved.
const BREAK: &str = "mutate-unchecked-index";

/// The provocation that finds it, and the line the log must carry.
const PROVOCATION: &str = "cap=forge";

/// What a kernel with [`BREAK`] in it prints on the way down.
const PANIC: &str = "KERNEL PANIC";

/// The exit code a boot that reached the end reports. QEMU reports
/// `(value << 1) | 1`, so the kernel's `Exit::Success(0x10)` arrives as this.
/// Unit: none — a process exit status.
const OK: i32 = 33;

/// The line the frame prints when it has selected a generation.
const SELECTED: &str = "  generation    selected";

/// The line it prints when it cannot.
const REFUSED: &str = "FAIL: the generation: no module this machine was offered folds";

/// `cargo xtask rollback`.
///
/// # Errors
///
/// Any of the seven steps below failing to be what it says it is. Each one
/// reports what it expected and what it got, because the reader of a red run
/// here is somebody deciding whether the rollback story is broken or the tree
/// is.
pub fn rollback() -> Result<(), String> {
    let dir = crate::target_dir().join("rollback");
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;

    println!("[1/7] the generation this machine is");
    let before = generation::pack(&[])?;
    let kept = dir.join(format!("{}.fcm", hex(&before.root)));
    std::fs::write(&kept, &before.bytes).map_err(|e| format!("writing {}: {e}", kept.display()))?;
    describe("before", &before);
    println!("  kept at     {}", crate::relative(&kept));

    println!("\n[2/7] break it — the same source, a frame with a real defect in it");
    let broken = generation::pack(&[BREAK])?;
    describe("broken", &broken);
    moved(&before, &broken)?;

    println!("\n[3/7] the broken generation is broken, and not merely different");
    let (code, log) = crate::boot_captured(Some(PROVOCATION), &[BREAK])?;
    if code == Some(OK) {
        return Err(format!(
            "`{PROVOCATION}` passed on the generation built with `{BREAK}`.\n\n\
             There is nothing to roll back from: a generation that differs and works is a\n\
             different machine, not a broken one, and the rest of this command would be\n\
             demonstrating a selection rather than a rollback."
        ));
    }
    if !log.contains(PANIC) {
        return Err(format!(
            "the broken generation went red and not for the reason it was built to.\n\n\
             The log does not contain `{PANIC}`. A boot that fails some other way\n\
             satisfies the exit code and proves nothing about `{BREAK}`."
        ));
    }
    println!("  broken      `{PROVOCATION}` takes the machine down with `{PANIC}`");

    println!("\n[4/7] and the generation being rolled back to is not broken");
    match crate::boot_captured(Some(PROVOCATION), &[])?.0 {
        Some(OK) => println!("  intact      the same provocation is survived"),
        other => {
            return Err(format!(
                "`{PROVOCATION}` fails on a generation with no defect in it (qemu exited \
                 {other:?}), so the red boot above says nothing about `{BREAK}`. Fix the \
                 tree first."
            ));
        }
    }

    println!("\n[5/7] roll back from the boot menu — both generations offered, one token");
    let menu = [as_arg(&before.module)?, as_arg(&broken.module)?];
    let restored = boot_selecting(&before.root, &menu)?;
    let elsewhere = boot_selecting(&broken.root, &menu)?;
    if elsewhere.root == restored.root {
        return Err(
            "the same menu answered two different tokens with one generation, so the token \
             is not what selected it. That would make every comparison below a comparison \
             of a file with itself."
                .into(),
        );
    }
    println!("  selects     `f.root=` picks each of the two out of one menu");
    refuses(&menu)?;

    println!("\n[6/7] bit-identical, and here is what that is taken over");
    let after = generation::pack(&[])?;
    identical(&before, &after, &restored, &kept)?;

    println!("\n[7/7] and the digest comparison earns its place");
    let caught = tampered(&dir, &before)?;

    println!(
        "\nrollback: ok\n\
        \x20 the generation was broken with `{BREAK}`, `{PROVOCATION}` took the machine\n\
        \x20 down, `f.root={}` selected the previous one out of a menu of two, and\n\
        \x20 what came back matched on all three: the root, the {} bytes of the module\n\
        \x20 as the loader delivered them, and the generation rebuilt from source.\n\
        \x20 The tampered module folds to the same root and is caught by the digest\n\
        \x20 and by `f-assembler` naming member {caught}, which is why the root alone\n\
        \x20 is not the comparison.",
        &hex(&before.root)[..16],
        before.bytes.len()
    );
    Ok(())
}

/// What one packed generation is, printed so two runs are two comparable logs.
fn describe(what: &str, packed: &Packed) {
    println!("  {what:<11} root   {}", hex(&packed.root));
    println!(
        "  {:<11} module {}  ({} bytes)",
        "",
        hex(&f_hash::sha256(&packed.bytes)),
        packed.bytes.len()
    );
    println!("  {:<11} frame  {}", "", hex(&packed.frame));
    for (name, address) in &packed.components {
        println!("  {:<11} {name:<14} {}", "", hex(address));
    }
}

/// The break moved the frame and nothing else.
///
/// Asserted rather than assumed, because the whole of step six rests on it: if
/// the defect had reached a component leaf as well, "the frame is what broke"
/// would be a sentence this command prints and has not checked, and a rollback
/// that restored the wrong thing would still look green.
fn moved(before: &Packed, broken: &Packed) -> Result<(), String> {
    if before.root == broken.root {
        return Err(format!(
            "the generation built with `{BREAK}` has the same root as the one without it.\n\n\
             Either the feature is not reaching the frame image or the frame leaf is not \
             in the root, and both make every step below meaningless."
        ));
    }
    if before.components != broken.components {
        return Err("the defect reached a component leaf as well as the frame. It is declared in \
             `kernel/Cargo.toml` and `compile_here` passes it to the frame build alone, so \
             this is the build leaking a feature rather than a rollback finding anything."
            .into());
    }
    let (was, now) = (tree_of(before)?, tree_of(broken)?);
    let left = f_generation::Tree::check(&was)
        .map_err(|why| format!("the kept module's tree: {}", why.message()))?;
    let right = f_generation::Tree::check(&now)
        .map_err(|why| format!("the broken module's tree: {}", why.message()))?;

    // `divergence` reports the frame as the leaf it is — it is the generation
    // node's first child and has no variant of its own — so what makes this
    // finding *the frame* is that the name is the one the tree's own frame leaf
    // carries. Asked of the tree rather than spelled `"kernel"` here, because a
    // literal would be this command's second opinion about `user/generation.toml`.
    let frame_name = left.frame().name;
    match f_generation::divergence(&left, &right) {
        Some(f_generation::Divergence::Leaf { left: a, right: b })
            if a.name == frame_name && b.name == frame_name =>
        {
            println!("  moved       one leaf, and it is the frame — every component stood still");
            Ok(())
        }
        other => Err(format!(
            "the break was supposed to move the frame leaf and nothing else, and the \
             comparison names {other:?} instead.\n\n\
             `generation/src/diff.rs` descends leaves before roots, so this is the \
             comparison naming what actually moved. A defect that reaches more than the \
             frame is a defect this command cannot reason about."
        )),
    }
}

/// The record tree out of a packed module, through the reader that reads it at
/// boot.
///
/// Asked of `f_abi::boot::Module` rather than sliced out of the bytes here.
/// This command is not a second reader of the layout — the spec's stated
/// reversal for the whole boot-module arrangement is a second reader appearing
/// anywhere — so every offset below comes from the type that owns it.
fn tree_of(packed: &Packed) -> Result<Vec<u8>, String> {
    let module = f_abi::boot::Module::read(&packed.bytes)
        .map_err(|why| format!("the module this command packed does not decode ({why:#x})"))?;
    Ok(module.tree().to_vec())
}

/// One boot, offering a menu and naming one generation on the command line.
///
/// Returns what the frame said it selected, read out of the serial log — which
/// is the only place it can be read from, and is why the frame prints a root
/// and a digest rather than only a decision.
fn boot_selecting(root: &[u8; 32], menu: &[String]) -> Result<Reported, String> {
    let append = format!("f.root={}", hex(root));
    let (code, log) = crate::boot_carrying(&append, menu, &[])?;
    if code != Some(OK) {
        return Err(format!(
            "the boot selecting `{append}` exited {code:?} rather than {OK}.\n\n\
             The serial log is above. A machine that was asked for a generation and could \
             not be it is a failure and not a warning — `kernel/src/generation.rs` says why."
        ));
    }
    let reported = Reported::read(&log).ok_or_else(|| {
        format!(
            "the boot selecting `{append}` exited {OK} and printed no selection block.\n\n\
             That is worse than a refusal: the machine was handed a token, said nothing \
             about it, and reported success. Either the frame did not read the command \
             line or `{SELECTED}` has moved."
        )
    })?;
    if reported.root != *root {
        return Err(format!(
            "the machine was asked for {} and reports having selected {}.",
            hex(root),
            hex(&reported.root)
        ));
    }
    Ok(reported)
}

/// A token no module answers to is refused rather than defaulted.
///
/// The other half of *the token selects*: a machine that fell back to whatever
/// module it happened to have would pass every other check in this file while
/// booting something nobody asked for, which is the exact failure a rollback
/// test exists to catch — the operator believes they rolled back and the
/// evidence says nothing.
fn refuses(menu: &[String]) -> Result<(), String> {
    let append = format!("f.root={}", "0".repeat(64));
    let (code, log) = crate::boot_carrying(&append, menu, &[])?;
    if code == Some(OK) {
        return Err(
            "a root no offered module carries booted to success. The frame is defaulting to \
             something rather than refusing, so `f.root=` is a decoration and not a \
             selection."
                .into(),
        );
    }
    if !log.contains(REFUSED) {
        return Err(format!(
            "the boot for an unknown root went red and did not say why: the log carries no \
             `{REFUSED}`. A red exit code alone would be satisfied by any failure."
        ));
    }
    println!("  refuses     a root nothing on the menu carries is refused by name");
    Ok(())
}

/// The three comparisons, made and named.
fn identical(
    before: &Packed,
    after: &Packed,
    restored: &Reported,
    kept: &Path,
) -> Result<(), String> {
    // One. Necessary, and the one a hash comparison already gives.
    if restored.root != before.root {
        return Err("the machine folded a different root than the one that was packed".into());
    }
    println!("  root        {}  — necessary, and not sufficient", hex(&restored.root));

    // Two. The bit-identity clause, taken on the machine over the bytes that
    // arrived rather than over the file that was meant to.
    let expected = f_hash::sha256(&before.bytes);
    if restored.digest != expected {
        return Err(format!(
            "the root matched and the bytes did not.\n\n\
             The machine digested {} over the module the loader delivered; the module \
             packed before the break digests to {}. That is exactly the case a root \
             comparison cannot see, and the reason this command takes both.",
            hex(&restored.digest),
            hex(&expected)
        ));
    }
    println!(
        "  bytes       {}  — every byte of tree, framing and {} component file(s)",
        hex(&restored.digest),
        before.components.len()
    );

    // Three. Rebuilt from source after the break, which is what makes this a
    // rollback rather than the re-selection of a file nobody disturbed.
    let kept_bytes = std::fs::read(kept).map_err(|e| format!("reading {}: {e}", kept.display()))?;
    if kept_bytes != before.bytes {
        return Err(format!("{} is not the bytes this command wrote to it", crate::relative(kept)));
    }
    for (what, left, right) in [
        ("root", before.root.as_slice(), after.root.as_slice()),
        ("frame leaf", before.frame.as_slice(), after.frame.as_slice()),
        ("module", before.bytes.as_slice(), after.bytes.as_slice()),
    ] {
        if left != right {
            return Err(format!(
                "the generation rebuilt from source after the break differs from the one \
                 packed before it, at the {what}.\n\n\
                 The rollback selected a file that was already on the disk, so it would \
                 have passed the two comparisons above regardless. This is the one that \
                 says the generation is restorable and not merely re-selectable, and it \
                 has failed — which is a reproduction failure and belongs to `E2-P06`."
            ));
        }
    }
    if before.components != after.components {
        return Err("a component leaf moved across the break and back".into());
    }
    println!(
        "  rebuilt     the same source after the break packs the same {} bytes",
        after.bytes.len()
    );
    Ok(())
}

/// The module that passes a root comparison and fails a byte comparison.
///
/// Built by asking `f_abi::boot::Module` where a component file sits and
/// flipping one byte at the end of it — inside the image, past the manifest
/// record. The record tree is untouched, so the leaves are untouched, so the
/// fold is untouched — which is the whole point: this is the module a rollback
/// that compared roots would accept as the generation it rolled back to.
///
/// Returns the member index `f-assembler` named, so the caller can print it.
fn tampered(dir: &Path, before: &Packed) -> Result<u16, String> {
    let module = f_abi::boot::Module::read(&before.bytes)
        .map_err(|why| format!("the kept module does not decode ({why:#x})"))?;
    let file = module.file(0).ok_or("the kept module carries no component file")?;
    // The offset is asked of the reader rather than recomputed here, for the
    // reason `tree_of` gives: a second reader of this layout in `xtask` is the
    // reversal the spec names for the whole arrangement. `as_ptr` is safe and
    // the subtraction is exact — `file` is a subslice of `before.bytes`.
    let at = file.as_ptr() as usize - before.bytes.as_ptr() as usize;
    let last = at + file.len() - 1;

    // **One byte of the image, and not of the manifest record in front of it.**
    // That distinction was found rather than designed: filling the whole file
    // with a run of one byte made it stop being a manifest, and `f-assembler`
    // answered `Component(0, NotAManifest)` — a shape refusal from the codec,
    // before the content check this step is about had a chance to run. Which is
    // correct of the assembler and useless as a demonstration, because a module
    // whose component file is not even a manifest is caught by a check that was
    // never in doubt.
    //
    // The corruption worth being able to catch is the quiet one: a component
    // file whose record still reads, whose name still matches its leaf, and
    // whose *image* is not the image the leaf's address names. That is what a
    // flipped byte in the last position is, and it is what a device that lied
    // about a barrier leaves behind.
    let mut bytes = before.bytes.clone();
    bytes[last] ^= 0xFF;
    let path = dir.join("tampered.fcm");
    std::fs::write(&path, &bytes).map_err(|e| format!("writing {}: {e}", path.display()))?;

    // On the host: the root stands still and the digest moves.
    let examined = f_generation::select::examine(&bytes)
        .map_err(|why| format!("the tampered module no longer decodes: {}", why.message()))?;
    if examined.root != before.root {
        return Err(
            "the tampered module folds to a different root, so it demonstrates nothing: the \
             swap was supposed to leave the record tree alone. Either the tree overlaps the \
             file extent or `Module::file` is answering with the wrong bytes."
                .into(),
        );
    }
    if examined.digest == f_hash::sha256(&before.bytes) {
        return Err("the tampered module digests to the same value as the honest one".into());
    }

    // And `f-assembler` names the member, which is the third and finest answer
    // — the one the frame deliberately does not duplicate. RFC 0066.
    let named = match f_assembler::Assembly::instantiate(&before.root, &bytes) {
        Err(f_assembler::Refusal::Content(index)) => index,
        other => {
            return Err(format!(
                "the assembler was supposed to refuse the tampered module for its content \
                 and answered {other:?} instead.\n\n\
                 That check — every file's SHA-256 is the address its leaf names — is the \
                 only thing in this system that catches a swapped file by *name*, and a \
                 rollback whose module passed it would have no way to say which component \
                 was wrong."
            ));
        }
    };

    // And on the machine, which is the half that matters: offered alone under
    // the honest root, the frame selects it — a root comparison accepts it —
    // and reports a digest that is not the one that was packed.
    let menu = [as_arg(&path)?];
    let append = format!("f.root={}", hex(&before.root));
    let (code, log) = crate::boot_carrying(&append, &menu, &[])?;
    if code != Some(OK) {
        return Err(format!(
            "the boot offered only the tampered module exited {code:?} rather than {OK}.\n\n\
             It was supposed to *succeed*: the frame selects by the fold, the fold is the \
             honest root, and the whole demonstration is that success here is not enough."
        ));
    }
    let reported = Reported::read(&log)
        .ok_or("the boot offered the tampered module printed no selection block")?;
    if reported.root != before.root {
        return Err("the frame did not select the tampered module under the honest root".into());
    }
    if reported.digest == f_hash::sha256(&before.bytes) {
        return Err(
            "the machine digested the tampered module to the honest module's value, which \
             would mean the frame is hashing something other than the bytes it was handed."
                .into(),
        );
    }
    println!(
        "  caught      the frame selected it under {}… and digested {}…",
        &hex(&reported.root)[..16],
        &hex(&reported.digest)[..16]
    );
    println!(
        "  so          a rollback comparing roots would have accepted it; the digest and \
         `f-assembler` name member {named}"
    );
    Ok(named)
}

/// What the frame reported about the generation it selected.
///
/// Read out of the serial log by the two labels the frame prints, and not by
/// position: a block that gained a line would otherwise be parsed off by one
/// and the comparison would be between two things nobody chose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Reported {
    /// The fold the machine computed over the module it selected.
    /// Unit: bytes, exactly 32 — a SHA-256.
    root: [u8; 32],
    /// The SHA-256 the machine computed over every byte of that module.
    /// Unit: bytes, exactly 32 — a SHA-256.
    digest: [u8; 32],
}

impl Reported {
    /// Parse the block, or `None` if the log does not carry one.
    fn read(log: &str) -> Option<Self> {
        if !log.contains(SELECTED) {
            return None;
        }
        Some(Self {
            root: labelled(log, "    root        ")?,
            digest: labelled(log, "    bytes       ")?,
        })
    }
}

/// The thirty-two bytes printed after a label, or nothing.
fn labelled(log: &str, label: &str) -> Option<[u8; 32]> {
    let line = log.lines().find(|line| line.starts_with(label))?;
    let digits = line.strip_prefix(label)?.trim();
    // Lower case only, which `from_str_radix` alone would not hold: it takes
    // `1A` and `1a` as one number, and then a frame that started shouting its
    // hashes would go on comparing equal while printing something no other tool
    // in this tree emits. `f_abi::boot::Selection::parse` refuses upper case for
    // the same reason — R04, and a canonical form is only canonical if there is
    // one of it — so the reader of the frame's output holds the same line.
    if digits.len() != 64
        || !digits.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    let mut out = [0u8; 32];
    for (n, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(digits.get(2 * n..2 * n + 2)?, 16).ok()?;
    }
    Some(out)
}

/// A module path as the emulator takes it.
fn as_arg(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("the boot module path {} is not valid UTF-8", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A serial log with a selection block in it, the way a boot prints one.
    fn log(root: &str, digest: &str) -> String {
        format!(
            "  timer         100 ticks at 1000 Hz\n\
             {SELECTED} — one offered module folds to the root asked for\n    \
             root        {root}\n    \
             bytes       {digest}\n    \
             module      at index 5 of the loader's list; 2 generation(s) offered\n\
             M0 ok\n"
        )
    }

    #[test]
    fn the_two_hashes_are_read_by_their_labels_and_not_by_position() {
        // By label, because a block that gained a line would otherwise be parsed
        // off by one and the comparison would be between two things nobody
        // chose — which is a green run for the wrong reason, and this whole
        // command exists so that there is not one of those.
        let reported = Reported::read(&log(&"1a".repeat(32), &"2b".repeat(32)))
            .expect("a log this test wrote");
        assert_eq!(reported.root, [0x1a; 32]);
        assert_eq!(reported.digest, [0x2b; 32]);
    }

    #[test]
    fn a_log_with_no_selection_block_reads_as_nothing_rather_than_as_zero() {
        // The difference matters: `None` makes the caller fail with *the machine
        // said nothing about the token it was handed*, and a zeroed pair would
        // make it fail with *the roots differ*, which names the wrong thing.
        assert_eq!(Reported::read("  timer 100 ticks\nM0 ok\n"), None);
        assert_eq!(Reported::read(""), None);
    }

    #[test]
    fn a_truncated_or_upper_case_hash_is_not_read_as_a_shorter_one() {
        // `f_abi::boot::Selection` refuses upper case so that two spellings
        // cannot select one generation, and the reader of the frame's own output
        // holds the same line: a hash printed one way and parsed another is a
        // hash two people compare by eye and disagree about.
        assert_eq!(Reported::read(&log(&"1a".repeat(31), &"2b".repeat(32))), None);
        assert_eq!(Reported::read(&log(&"1A".repeat(32), &"2b".repeat(32))), None);
    }
}
