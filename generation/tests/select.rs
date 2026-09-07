// SPDX-License-Identifier: Apache-2.0 OR MIT
//! A menu of boot modules, and which one `f.root=` picks out of it.
//!
//! # What this file exists to assert, and it is not "the right module was found"
//!
//! It is `E2-P07`'s strict clause: **a root that matches is necessary and is not
//! the same as the module being bit-identical.** The fold covers the record
//! tree — each leaf's name and the content address it names — and it does not
//! cover the component files that travel in the same module. So a module
//! carrying a correct tree beside a file that is not the one its leaf names
//! folds to the same root, and every root comparison in this system passes over
//! it. [`the_digest_moves_where_the_root_does_not`] is that module, built on
//! purpose, and the assertion is that exactly one of the two numbers moves.
//!
//! The fixture is written out here rather than taken from `xtask`'s compiler,
//! for `divergence.rs`'s reason about its own: a selection test that took its
//! modules from the writer would be testing the writer against itself. The
//! layout is `f_abi::boot`'s own constants, so a change to the format stops this
//! file rather than passing through it.

use f_abi::boot::{MODULE_HEAD_BYTES, MODULE_MAGIC};
use f_abi::manifest::NAME_MAX;
use f_abi::store::{Generation, Leaf, Route, Topology, node};
use f_generation::record::Tree;
use f_generation::select::{Rejected, examine, is_module, select};
use f_generation::{Refusal, fold};

fn padded(text: &str) -> [u8; NAME_MAX] {
    let mut out = [0u8; NAME_MAX];
    out[..text.len()].copy_from_slice(text.as_bytes());
    out
}

fn component(name: &str, fill: u8) -> Leaf {
    Leaf { kind: node::COMPONENT, hash: [fill; 32], name: padded(name) }
}

/// A record tree, encoded the way `f_abi::store` says and nothing else.
fn tree(frame_fill: u8, members: &[Leaf], routes: &[Route]) -> Vec<u8> {
    let head = Topology { members: members.len() as u16, routes: routes.len() as u16 };
    let mut out = Vec::new();
    out.extend_from_slice(&Generation::to_bytes());
    out.extend_from_slice(
        &Leaf { kind: node::BYTES, hash: [frame_fill; 32], name: padded("kernel") }.to_bytes(),
    );
    out.extend_from_slice(&head.to_bytes());
    for one in routes {
        out.extend_from_slice(&one.to_bytes());
    }
    for one in members {
        out.extend_from_slice(&one.to_bytes());
    }
    out
}

/// A boot module, through `f_abi::boot`'s own constants.
fn module(tree: &[u8], files: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&MODULE_MAGIC.to_le_bytes());
    out.extend_from_slice(&(tree.len() as u32).to_le_bytes());
    out.extend_from_slice(&(files.len() as u32).to_le_bytes());
    for file in files {
        out.extend_from_slice(&(file.len() as u32).to_le_bytes());
    }
    out.extend_from_slice(tree);
    for file in files {
        out.extend_from_slice(file);
    }
    out
}

/// One generation: two components and one route between them.
fn generation(frame_fill: u8, files: &[Vec<u8>]) -> Vec<u8> {
    let members = [component("store", 1), component("virtio-blk", 2)];
    let routes = [Route { component: 0, source: 1, capability: padded("block") }];
    module(&tree(frame_fill, &members, &routes), files)
}

fn files() -> Vec<Vec<u8>> {
    vec![vec![0xA1; 24], vec![0xB2; 40]]
}

fn root_of(module: &[u8]) -> [u8; 32] {
    examine(module).expect("a module this file wrote").root
}

#[test]
fn the_token_picks_one_module_out_of_the_ones_the_loader_offered() {
    let old = generation(0xF0, &files());
    let new = generation(0xF1, &files());
    assert_ne!(root_of(&old), root_of(&new), "two frames must be two generations");

    // The menu, as a loader presents it: `user/init`'s flat image first — no
    // magic of ours, skipped in silence — then both generations.
    let init = vec![0u8; 64];
    let offered: [&[u8]; 3] = [&init, &new, &old];

    let back = select(&root_of(&old), offered.iter().copied());
    assert_eq!(back.at, Some(2), "the rollback selects the module it named, not the newest");
    assert_eq!(back.offered, 2, "the flat image is not a boot module and is not counted");
    assert_eq!(back.rejected, 0);
    assert_eq!(back.found.expect("a selection").root, root_of(&old));

    let forward = select(&root_of(&new), offered.iter().copied());
    assert_eq!(forward.at, Some(1), "and the same menu selects the other one by its own root");
}

#[test]
fn a_root_no_offered_module_carries_selects_nothing_rather_than_the_nearest() {
    let offered: [&[u8]; 1] = [&generation(0xF0, &files())];
    let chosen = select(&[0x77; 32], offered.iter().copied());
    assert_eq!(chosen.at, None);
    assert!(chosen.found.is_none());
    assert_eq!(chosen.offered, 1, "it was a boot module; it was simply not this one");
    assert_eq!(chosen.rejected, 0, "and it was not refused — nothing was wrong with it");
}

#[test]
fn a_damaged_module_is_counted_and_the_others_still_boot() {
    // A menu that lists four generations and one of them is damaged should
    // still boot the other three. That is the whole reason a menu has more than
    // one line, and a search that stopped at the first refusal would take it
    // away.
    let good = generation(0xF0, &files());
    let mut torn = generation(0xF1, &files());
    let last = torn.len() - 1;
    torn.truncate(last);

    let offered: [&[u8]; 2] = [&torn, &good];
    let chosen = select(&root_of(&good), offered.iter().copied());
    assert_eq!(chosen.at, Some(1));
    assert_eq!(chosen.offered, 2);
    assert_eq!(chosen.rejected, 1);
    assert!(matches!(chosen.why, Some(Rejected::Module(_))), "a short module, named as one");

    // And a module whose head decodes and whose tree does not is refused by the
    // other codec, so the two refusals stay two answers.
    let mut bent = generation(0xF0, &files());
    bent[MODULE_HEAD_BYTES + 2 * 4] ^= 0xFF;
    assert!(matches!(examine(&bent), Err(Rejected::Tree(Refusal::Node(_)))));
}

#[test]
fn the_digest_moves_where_the_root_does_not() {
    // The clause `E2-P07` exists to be strict about. Two modules, one record
    // tree, and one component file's bytes replaced by something the tree does
    // not name: the leaves are untouched, so the fold is untouched, so a
    // comparison over roots sees two identical generations.
    let honest = generation(0xF0, &files());
    let mut swapped = files();
    swapped[1] = vec![0xC3; 40];
    let tampered = generation(0xF0, &swapped);

    assert_eq!(honest.len(), tampered.len(), "the same shape, so only content moved");
    assert_ne!(honest, tampered, "and the bytes did move");

    let a = examine(&honest).expect("a module this file wrote");
    let b = examine(&tampered).expect("a module this file wrote");
    assert_eq!(a.root, b.root, "the root is blind to it, which is the point");
    assert_ne!(a.digest, b.digest, "and the digest is not");
    assert_eq!(a.members, b.members);
    assert_eq!(a.bytes, b.bytes);

    // Which is why a rollback that compared roots alone would accept the
    // tampered module as the generation it rolled back to: `select` hands it
    // back under the honest root.
    let offered: [&[u8]; 1] = [&tampered];
    let chosen = select(&a.root, offered.iter().copied());
    assert_eq!(chosen.at, Some(0), "selected under a root whose bytes it does not carry");
    assert_ne!(
        chosen.found.expect("a selection").digest,
        a.digest,
        "and the digest is the one number that says so"
    );
}

#[test]
fn the_digest_is_over_the_whole_module_and_not_over_the_tree() {
    // Stated as a test rather than as a comment, because a digest taken over
    // the tree would pass every other assertion in this file and would be
    // exactly as blind as the root.
    let honest = generation(0xF0, &files());
    let examined = examine(&honest).expect("a module this file wrote");
    assert_eq!(examined.digest, f_hash::sha256(&honest));
    assert_ne!(examined.digest, f_hash::sha256(&tree(0xF0, &[], &[])));
    assert_eq!(examined.bytes, honest.len());
}

#[test]
fn nothing_that_is_not_a_boot_module_is_examined() {
    // `user/init`'s flat image and every component file arrive on the same
    // module list. A frame that refused what it did not recognise would refuse
    // every boot this tree has ever run, so the rule is magic and the direction
    // is skip.
    assert!(!is_module(&[]));
    assert!(!is_module(&[0u8; 4]));
    assert!(!is_module(&[0xFFu8; 4096]));
    assert!(is_module(&generation(0xF0, &files())));

    let mut nearly = generation(0xF0, &files());
    nearly[7] ^= 0x01;
    assert!(!is_module(&nearly), "one bit of the magic is enough to not be one");
}

#[test]
fn the_root_this_selection_folds_is_the_one_the_fold_produces() {
    // The join. `examine` must not have a fold of its own — that would be the
    // second implementation the whole boot-module arrangement exists to refuse
    // — so the number it reports is checked against `fold::root` over the same
    // tree, taken the long way round.
    let bytes = generation(0xF0, &files());
    let head = MODULE_HEAD_BYTES + 2 * 4;
    let tree_bytes = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
    let checked = Tree::check(&bytes[head..head + tree_bytes]).expect("a tree this file wrote");
    assert_eq!(examine(&bytes).expect("a module").root, fold::root(&checked));
}
