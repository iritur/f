// SPDX-License-Identifier: Apache-2.0 OR MIT
//! One worked tree, and every way it can be wrong.
//!
//! The fixture is the same tree `user/generation.toml` describes — four
//! components and two routes — so that a reader who changes one has somewhere
//! obvious to look for the other, and so that the properties asserted here are
//! asserted about the shape the repository actually compiles.
//!
//! Every mutation below is a *one-byte* change wherever a one-byte change is
//! possible. A fold that hashed a summary rather than the encoding would pass a
//! test that changed a whole field and fail these, which is the point.

use f_abi::manifest::NAME_MAX;
use f_abi::store::{Generation, Leaf, Route, Topology, node};
use f_generation::fold;
use f_generation::record::{Refusal, Tree};

/// A NUL-padded name.
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

/// Encode a record tree the way `xtask/src/generation.rs` does, with the same
/// codec. Written out here rather than shared, because a fixture that used the
/// compiler's own encoder would test the two against each other and neither
/// against the format.
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

/// The four members, in canonical order: bytewise on the zero-padded name.
fn members() -> Vec<Leaf> {
    vec![
        component("store", 1),
        component("virtio-blk", 2),
        component("virtio-gpu", 3),
        component("virtio-net", 4),
    ]
}

/// Two routes, in canonical order: (component index, capability name).
fn routes() -> Vec<Route> {
    vec![route(0, 1, "block"), route(0, 3, "net")]
}

fn worked() -> Vec<u8> {
    encode(&frame(), &members(), &routes())
}

fn root_of(bytes: &[u8]) -> [u8; 32] {
    fold::root(&Tree::check(bytes).expect("the worked tree is canonical"))
}

#[test]
fn the_same_tree_folded_twice_is_the_same_root() {
    let bytes = worked();
    assert_eq!(root_of(&bytes), root_of(&bytes));
    // And a second encoding of the same expression, built from scratch, is the
    // same bytes — which is the half that would still be true if the fold were
    // wrong, and the half that would be false if the encoder read anything it
    // was not handed.
    assert_eq!(bytes, worked());
}

#[test]
fn one_byte_of_any_leaf_changes_the_root() {
    let before = root_of(&worked());

    let mut moved = members();
    moved[2].hash[31] ^= 1;
    assert_ne!(root_of(&encode(&frame(), &moved, &routes())), before);

    let mut other_frame = frame();
    other_frame.hash[0] ^= 1;
    assert_ne!(root_of(&encode(&other_frame, &members(), &routes())), before);
}

#[test]
fn one_byte_of_any_name_changes_the_root() {
    let before = root_of(&worked());

    // `store` becomes `stort`, which is still canonically first.
    let mut renamed = members();
    renamed[0].name = padded("stort");
    assert_ne!(root_of(&encode(&frame(), &renamed, &routes())), before);

    let mut other_frame = frame();
    other_frame.name = padded("kernal");
    assert_ne!(root_of(&encode(&other_frame, &members(), &routes())), before);
}

#[test]
fn one_byte_of_any_route_changes_the_root() {
    let before = root_of(&worked());

    let mut source_moved = routes();
    source_moved[0].source = 2;
    assert_ne!(root_of(&encode(&frame(), &members(), &source_moved)), before);

    let mut capability_moved = routes();
    capability_moved[1].capability = padded("nel");
    assert_ne!(root_of(&encode(&frame(), &members(), &capability_moved)), before);
}

#[test]
fn two_members_swapped_change_the_root_because_the_list_is_ordered() {
    // The names stay in canonical order and the *hashes* trade places, which is
    // the swap a set-of-components model would not notice. It is a different
    // system: `virtio-blk` is now the bytes `virtio-gpu` used to be.
    let mut swapped = members();
    swapped.swap(1, 2);
    swapped[1].name = padded("virtio-blk");
    swapped[2].name = padded("virtio-gpu");
    assert_ne!(root_of(&encode(&frame(), &swapped, &routes())), root_of(&worked()));
}

#[test]
fn a_non_canonical_source_is_refused_rather_than_reordered() {
    let mut out_of_order = members();
    out_of_order.swap(0, 3);
    let bytes = encode(&frame(), &out_of_order, &routes());
    assert_eq!(Tree::check(&bytes).map(|_| ()), Err(Refusal::Order));

    let mut backwards = routes();
    backwards.swap(0, 1);
    let bytes = encode(&frame(), &members(), &backwards);
    assert_eq!(Tree::check(&bytes).map(|_| ()), Err(Refusal::RouteOrder));
}

#[test]
fn a_duplicate_is_refused_rather_than_merged_or_dropped() {
    let mut twice = members();
    twice[1].name = padded("store");
    // Still sorted, and still two components with one name.
    let bytes = encode(&frame(), &twice, &routes());
    assert_eq!(Tree::check(&bytes).map(|_| ()), Err(Refusal::DuplicateName));

    let same = vec![route(0, 1, "block"), route(0, 2, "block")];
    let bytes = encode(&frame(), &members(), &same);
    assert_eq!(Tree::check(&bytes).map(|_| ()), Err(Refusal::DuplicateRoute));
}

#[test]
fn a_route_index_that_names_nothing_is_refused() {
    let dangling = vec![route(0, 9, "block")];
    let bytes = encode(&frame(), &members(), &dangling);
    assert_eq!(Tree::check(&bytes).map(|_| ()), Err(Refusal::Dangling));

    let backwards = vec![route(9, 0, "block")];
    let bytes = encode(&frame(), &members(), &backwards);
    assert_eq!(Tree::check(&bytes).map(|_| ()), Err(Refusal::Dangling));
}

#[test]
fn a_name_outside_its_alphabet_or_its_padding_is_refused() {
    for bad in ["Store", "sto re", "-store", "store-", ""] {
        let mut wrong = members();
        wrong[0].name = padded(bad);
        let bytes = encode(&frame(), &wrong, &routes());
        assert_eq!(
            Tree::check(&bytes).map(|_| ()),
            Err(Refusal::Name),
            "`{bad}` was accepted as a name"
        );
    }

    // A byte hidden in the padding. It is hashed, because the field is
    // fixed-width, so it would change a root without changing anything a person
    // can read.
    let mut smuggled = members();
    smuggled[0].name[NAME_MAX - 1] = b'x';
    let bytes = encode(&frame(), &smuggled, &routes());
    assert_eq!(Tree::check(&bytes).map(|_| ()), Err(Refusal::Name));

    let mut capability = routes();
    capability[0].capability = padded("BLOCK");
    let bytes = encode(&frame(), &members(), &capability);
    assert_eq!(Tree::check(&bytes).map(|_| ()), Err(Refusal::Name));
}

#[test]
fn a_node_of_the_right_shape_in_the_wrong_place_is_refused() {
    // A component where the frame belongs: the same seventy-two bytes, a
    // different kind, and a tree that would otherwise fold happily.
    let bytes = encode(&component("kernel", 0xF0), &members(), &routes());
    assert_eq!(Tree::check(&bytes).map(|_| ()), Err(Refusal::Kind));

    let mut wrong = members();
    wrong[0].kind = node::BYTES;
    let bytes = encode(&frame(), &wrong, &routes());
    assert_eq!(Tree::check(&bytes).map(|_| ()), Err(Refusal::Kind));
}

#[test]
fn a_tree_that_is_not_the_length_its_counts_say_is_refused() {
    let bytes = worked();

    assert_eq!(Tree::check(&bytes[..bytes.len() - 1]).map(|_| ()), Err(Refusal::Length));
    assert_eq!(Tree::check(&[]).map(|_| ()), Err(Refusal::Length));

    // A trailing byte, which no reader would look at and every fold would miss.
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert_eq!(Tree::check(&trailing).map(|_| ()), Err(Refusal::Length));
}

#[test]
fn a_header_this_build_does_not_believe_is_refused_by_its_own_codec() {
    let mut bytes = worked();
    bytes[0] ^= 0xFF;
    assert!(
        matches!(Tree::check(&bytes), Err(Refusal::Node(_))),
        "a broken magic reached the checker rather than the codec"
    );
}

#[test]
fn the_root_covers_the_generation_node_and_not_only_its_children() {
    // The two children's hashes, folded by hand the way the rule says, must be
    // the root. A fold that dropped the generation node's header — or that
    // hashed the children in the other order — passes every test above and
    // fails this one.
    let bytes = worked();
    let tree = Tree::check(&bytes).expect("the worked tree is canonical");

    let mut state = f_hash::Sha256::new();
    state.update(&Generation::to_bytes());
    state.update(&fold::leaf(&tree.frame()));
    state.update(&fold::topology(&tree));
    assert_eq!(state.finish(), fold::root(&tree));

    // And the frame's leaf hash is not the content address it carries, which is
    // the distinction a reader is most likely to collapse.
    assert_ne!(fold::leaf(&tree.frame()), tree.frame().hash);
}
