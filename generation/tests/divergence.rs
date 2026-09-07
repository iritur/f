// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Two trees, and the thing the comparison must name.
//!
//! The fixture is `root.rs`'s, written out again rather than shared, for the
//! reason that file gives about its own encoder: a comparison test that took its
//! trees from the compiler would be testing the two against each other. What is
//! asserted here is not that a difference is *found* — a hash comparison finds
//! that — but that the right thing is **named**, which is the whole difference
//! between a job that reports a failure and a job that reports a cause. Every
//! test below would pass against a function that returned the wrong leaf if the
//! assertion were `is_some()`.

use f_abi::manifest::NAME_MAX;
use f_abi::store::{Generation, Leaf, Route, Topology, node};
use f_generation::diff::{Divergence, divergence};
use f_generation::record::Tree;

fn padded(text: &str) -> [u8; NAME_MAX] {
    let mut out = [0u8; NAME_MAX];
    out[..text.len()].copy_from_slice(text.as_bytes());
    out
}

fn component(name: &str, fill: u8) -> Leaf {
    Leaf { kind: node::COMPONENT, hash: [fill; 32], name: padded(name) }
}

fn route(component: u16, source: u16, capability: &str) -> Route {
    Route { component, source, capability: padded(capability) }
}

fn encode(frame: &Leaf, members: &[Leaf], routes: &[Route]) -> Vec<u8> {
    let head = Topology { members: members.len() as u16, routes: routes.len() as u16 };
    let mut out = Vec::new();
    out.extend_from_slice(&Generation::to_bytes());
    out.extend_from_slice(&frame.to_bytes());
    out.extend_from_slice(&head.to_bytes());
    for one in routes {
        out.extend_from_slice(&one.to_bytes());
    }
    for one in members {
        out.extend_from_slice(&one.to_bytes());
    }
    out
}

fn frame() -> Leaf {
    Leaf { kind: node::BYTES, hash: [0xF0; 32], name: padded("kernel") }
}

fn members() -> Vec<Leaf> {
    vec![
        component("store", 1),
        component("virtio-blk", 2),
        component("virtio-gpu", 3),
        component("virtio-net", 4),
    ]
}

fn routes() -> Vec<Route> {
    vec![route(0, 1, "block"), route(0, 3, "net")]
}

fn worked() -> Vec<u8> {
    encode(&frame(), &members(), &routes())
}

/// The name a divergence points at, trimmed, so an assertion reads as the
/// sentence the job prints.
fn named(divergence: &Divergence) -> String {
    let padded = divergence.named().expect("this divergence names something");
    let end = padded.iter().position(|b| *b == 0).unwrap_or(padded.len());
    String::from_utf8_lossy(&padded[..end]).into_owned()
}

fn compare(left: &[u8], right: &[u8]) -> Option<Divergence> {
    let a = Tree::check(left).expect("the left tree is canonical");
    let b = Tree::check(right).expect("the right tree is canonical");
    divergence(&a, &b)
}

#[test]
fn two_runs_of_one_tree_diverge_nowhere() {
    assert_eq!(compare(&worked(), &worked()), None);
}

#[test]
fn a_component_whose_content_moved_is_named_by_its_own_name() {
    // One byte of one component's content address, which is what a
    // non-reproducible component file looks like from here.
    let mut moved = members();
    moved[2].hash[31] ^= 1;
    let other = encode(&frame(), &moved, &routes());

    let found = compare(&worked(), &other).expect("one leaf moved");
    assert_eq!(named(&found), "virtio-gpu");
    assert!(found.names_an_input(), "a moved leaf is an input, not a defect in the fold");
}

#[test]
fn a_frame_whose_content_moved_is_named_before_any_component() {
    // The frame *and* a component, so that the descent's order is what is being
    // asserted rather than the only difference there is. The frame is the
    // generation node's first child and the leaf that carries debug
    // information, so it is the one a reader should be shown first.
    let mut moved = members();
    moved[0].hash[0] ^= 1;
    let mut other_frame = frame();
    other_frame.hash[0] ^= 1;
    let other = encode(&other_frame, &moved, &routes());

    let found = compare(&worked(), &other).expect("two leaves moved");
    assert_eq!(named(&found), "kernel");
}

#[test]
fn both_sides_are_carried_so_neither_run_is_called_the_deviation() {
    let mut moved = members();
    moved[1].hash = [0xAB; 32];
    let other = encode(&frame(), &moved, &routes());

    let Some(Divergence::Leaf { left, right }) = compare(&worked(), &other) else {
        panic!("a moved component leaf is a Leaf divergence");
    };
    assert_eq!(left.hash, [2u8; 32]);
    assert_eq!(right.hash, [0xAB; 32]);
    // And the comparison is symmetric in everything but which side is which: a
    // job that ran it the other way round must reach the same finding.
    let Some(Divergence::Leaf { left: back, right: forth }) = compare(&other, &worked()) else {
        panic!("the reversed comparison is still a Leaf divergence");
    };
    assert_eq!((back.hash, forth.hash), (right.hash, left.hash));
}

#[test]
fn a_missing_component_is_a_membership_and_not_a_leaf() {
    // The route naming it goes too, because a route to a component that is not
    // there is refused by `Tree::check` before this function ever sees it —
    // which is the checker doing its job and would make this test a test of the
    // checker instead. So the members differ and the routes differ, and what is
    // asserted is that the descent reports the *members* first: they are the
    // children and a route is a field.
    let mut short = members();
    short.pop();
    let other = encode(&frame(), &short, &routes()[..1]);

    let found = compare(&worked(), &other).expect("the member sets differ");
    assert!(matches!(found, Divergence::Membership { .. }));
    // The name is still carried, because *which* component is absent is the
    // whole of the finding.
    assert_eq!(named(&found), "virtio-net");
}

#[test]
fn a_renamed_component_is_a_membership_and_not_a_moved_leaf() {
    // Same content, same position, different name. There is no pair to compare
    // here — the two topologies name different things — and reporting it as a
    // leaf that moved would send a reader looking at bytes that are identical.
    let mut renamed = members();
    renamed[0].name = padded("stort");
    let other = encode(&frame(), &renamed, &routes());

    let found = compare(&worked(), &other).expect("the member sets differ");
    assert!(matches!(found, Divergence::Membership { .. }));
}

#[test]
fn a_route_that_moved_is_found_even_though_no_leaf_did() {
    // A route is a field of the topology and not a child of it, so this moves
    // the root while every content address under it stands still. Without the
    // route walk the descent would fall through to `Compiler` and blame the
    // fold for a difference in the source.
    let mut moved = routes();
    moved[1] = route(0, 2, "net");
    let other = encode(&frame(), &members(), &moved);

    let found = compare(&worked(), &other).expect("a route moved");
    assert!(matches!(found, Divergence::Route { index: 1, .. }), "{found:?}");
    assert_eq!(named(&found), "net");
}

#[test]
fn a_dropped_route_is_a_route_divergence_with_a_side_missing() {
    let mut short = routes();
    short.pop();
    let other = encode(&frame(), &members(), &short);

    let found = compare(&worked(), &other).expect("a route went");
    assert!(matches!(found, Divergence::Route { index: 1, right: None, .. }), "{found:?}");
}

#[test]
fn nothing_below_the_root_is_ever_reported_as_agreement() {
    // The property `divergence` is written around: `None` is taken from the
    // root and not from having run out of things to compare, so a pair that the
    // descent finds nothing in and whose roots differ is still a finding. There
    // is no way to build such a pair through the encoder — every field the fold
    // reads is a field the descent reads — so what is asserted is the reverse:
    // every difference the encoder *can* express is named by something other
    // than `Compiler`, which is what makes `Compiler` mean what it says.
    let mut every: Vec<Vec<u8>> = Vec::new();
    let mut moved = members();
    moved[3].hash[0] ^= 1;
    every.push(encode(&frame(), &moved, &routes()));
    let mut other_frame = frame();
    other_frame.name = padded("kernal");
    every.push(encode(&other_frame, &members(), &routes()));
    let mut moved_routes = routes();
    moved_routes[0] = route(0, 2, "block");
    every.push(encode(&frame(), &members(), &moved_routes));

    for other in every {
        let found = compare(&worked(), &other).expect("these differ");
        assert!(found.names_an_input(), "{found:?} blamed the fold for a difference in a tree");
    }
}
