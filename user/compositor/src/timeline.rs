// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The frame's client application: `claims/0033-scene/scene.toml` built as an
//! immediate-mode tree, one whole tree per frame, for `E3-B01`'s count.
//!
//! # Why a client lives in the compositor's crate
//!
//! It should not, and the reason it does is a fact about this build rather than
//! a design. The client of every compositor boot is the frame — `crate`'s own
//! comment and `kernel/src/compositor.rs` both say so — and the frame cannot be
//! host-tested: `kernel/Cargo.toml` turns its test harness off because two
//! crates would claim `panic_impl`. The scene a client builds has to be checked
//! against the file it claims to be, and that check is a host test or nothing.
//! This is the one crate the frame links that also runs on a host, so the
//! application is here and the frame calls it. Nothing in the component calls
//! it: the serve loop takes entries off a ring and never builds a tree, and the
//! image does not grow by a byte for this module being in the crate, because
//! nothing reachable from `component::start` names it.
//!
//! `crate`'s comment used to say *no client library: nothing in this crate
//! links the reconciler, because the side that produces deltas is the side that
//! would*. That is still the rule. What changed is that the side that produces
//! deltas is the frame and the frame's only host-testable crate is this one, so
//! the reconciler is re-exported here for the frame to *call*, and the component
//! never does.
//!
//! *What would reverse this:* a client that is not the frame — `E3-B01g`'s
//! remaining sentence, an unprivileged application holding the far end of a
//! scene ring — at which point this module moves into that application; or the
//! frame taking `f-scene` as a dependency of its own, at which point the
//! re-export goes and the scene stays here only if its test does.
//!
//! # Which frame, and why this one
//!
//! `E3-B02j` chose this scene for *representativeness rather than
//! reachability* and argued it from design section 13: an audio timeline at the
//! moment its playhead moves, **three dirty nodes of 995**. It is the most
//! defensible definition this tree has of an ordinary frame, and it was chosen
//! by a task that was not trying to make a crossing count small, which is the
//! property that matters here. RFC 0133 records the choice and what a skeptic
//! would pick instead.
//!
//! So the frame [`E3-B01`'s count](crate::routing::reported::CUT_FRAMES) is taken
//! over is a **warm** frame: the whole tree rebuilt, the playhead's transform
//! one step further along, and nothing else different. The cold frames that put
//! the tree into the compositor in the first place are not that frame — the
//! scene's own README says *the first frame of this scene is a different number
//! and is not what the claim's rows bound* — and they are not one frame either:
//! [`build_frames`] says why they cannot be.
//!
//! # What of the scene reaches the compositor, and what cannot
//!
//! Every node reaches it — all 995, with the kinds the census gives, hung where
//! the parts say — and every transform and every paint. Three things do not,
//! and each is a row of the scene's own `[unreachable]` table rather than a
//! choice made here:
//!
//! - **Geometry.** 645 paths over 35 729 segments. `SetPath` names a range of
//!   the channel's inline arena, the arena of a one-page channel is two
//!   kibibytes, one waveform of 512 segments is larger than that at any
//!   encoding, and the path encoding is `E3-B02c`'s and does not exist. So no
//!   node here carries a path; a draw node is a node with a paint and no
//!   outline.
//! - **Glyph runs.** 155 runs, 1 420 scalars. There is no opcode for text and no
//!   shaper (RFC 0082), so a run is a draw node whose content cannot be stated.
//! - **Effect declarations.** The two effect nodes exist; their estimate and
//!   saving travel on `SetEffect`, which the reconciler does not emit — its
//!   property list is transform, path and paint.
//!
//! **None of the three moves in the warm frame**, which is why the part that
//! reaches is still the frame the scene describes. The reconciler emits a
//! property delta only where a property differs, so content that is constant
//! across two frames costs nothing whether or not it could be expressed; what
//! moves in this frame is one transform, and a transform is expressed exactly.
//! The claim that rests on this is narrow and is stated in RFC 0133: *the warm
//! frame's deltas are the deltas the whole scene would cost*, and the cold
//! frames' are not.
//!
//! # No clock, no seed, no allocator
//!
//! Every node is written by arithmetic over its index and the playhead's
//! position, into a slice the caller owns. RFC 0004: the position is an integer
//! in 16.16 fixed point, and nothing here reads anything.

use f_abi::scene::{NO_NODE, SetPaint, SetTransform, kind};

pub use f_scene::reconcile::{Deltas, Node, Reconciler, Refusal};

/// How many nodes the scene has: `[census] nodes` in `claims/0033-scene/scene.toml`.
///
/// Also the size the frame gives its reconciler, and exactly this rather than
/// `f_scene::arena::NODES_MAX`: a tree of 995 in a reconciler of 1 024 would
/// leave room for a scene that is not this one to be built without a refusal.
/// The host test below reads the census and requires the two equal.
/// Unit: nodes.
pub const NODES: usize = 995;

/// How many parts the scene declares. Unit: parts.
pub const PARTS: usize = 12;

/// One part of the scene, with the per-kind split this module chose for it.
///
/// **The split is the one thing here that `scene.toml` does not state**, and it
/// is said rather than hidden: the file gives each part its node count and the
/// kinds it contains, and gives the per-kind totals for the whole scene, but not
/// per-kind counts per part. The split below is the one the parts' own notes
/// describe — *eight tracks of twelve clips: a rounded body, its stroke, a name
/// clipped to the body* — and it sums to the census, which the test holds. It
/// moves no crossing in the warm frame, which touches one node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Part {
    /// The part's name in `scene.toml`.
    pub name: &'static str,
    /// Its parent part's name, or the empty string for the root.
    pub parent: &'static str,
    /// Nodes of each kind, in `f_abi::scene::kind`'s order: transform, clip,
    /// layer, draw, effect, semantic. Unit: nodes.
    pub kinds: [u32; 6],
}

/// The twelve parts, in `scene.toml`'s order.
pub const SCENE: [Part; PARTS] = [
    Part { name: "surface", parent: "", kinds: [0, 0, 0, 1, 0, 1] },
    Part { name: "chrome_bar", parent: "surface", kinds: [1, 0, 0, 25, 0, 18] },
    Part { name: "track_headers", parent: "surface", kinds: [0, 0, 0, 32, 0, 17] },
    Part { name: "viewport", parent: "surface", kinds: [0, 1, 0, 0, 0, 0] },
    Part { name: "scroll", parent: "viewport", kinds: [1, 0, 0, 0, 0, 0] },
    Part { name: "ruler", parent: "scroll", kinds: [0, 0, 0, 264, 0, 0] },
    Part { name: "clip_bodies", parent: "scroll", kinds: [8, 96, 0, 352, 0, 0] },
    Part { name: "selection", parent: "scroll", kinds: [0, 0, 1, 1, 0, 0] },
    Part { name: "playhead", parent: "scroll", kinds: [1, 0, 0, 2, 0, 0] },
    Part { name: "inspector", parent: "surface", kinds: [1, 0, 1, 25, 2, 25] },
    Part { name: "status_line", parent: "surface", kinds: [0, 0, 0, 1, 0, 1] },
    Part { name: "canvas_model", parent: "viewport", kinds: [0, 0, 0, 0, 0, 117] },
];

/// Where the playhead's transform sits in a frame, in device pixels.
///
/// The frame ordinal times two, plus a start. Two pixels a frame is a playhead
/// crossing a 2 880-pixel timeline in a little under half a minute at sixty
/// hertz — a playing track, not an animation test. Any step that is not zero
/// makes the same frame; this one is written down so the boot log's frames are
/// the same frames on every run.
/// Unit: device pixels, scaled by 65 536.
#[must_use]
pub const fn playhead_x65536(frame: u64) -> i64 {
    let pixels = 1_200 + 2 * (frame % 800) as i64;
    pixels * 65_536
}

/// What [`build`] wrote: where each part is and which node the playhead is.
///
/// Parts are contiguous ranges of the pre-order, because a part is a subtree
/// and the builder writes a part whole before the next; the range is what lets
/// a test count a part's kinds without a second copy of the builder's shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Built {
    /// Each part's first slot and one past its last, in [`SCENE`]'s order.
    /// Unit: slots of the tree.
    pub parts: [(usize, usize); PARTS],
    /// The key of the playhead's transform. Unit: none — a node identifier.
    pub playhead: u32,
    /// How many nodes were written. Unit: nodes.
    pub written: usize,
}

/// The writer [`build`] uses. Keys are one past the slot, so pre-order and key
/// order are one order and a prefix of the tree is a prefix of the keys.
struct Out<'a> {
    tree: &'a mut [Node],
    at: usize,
}

impl Out<'_> {
    /// Append one node and answer its key.
    fn node(&mut self, parent: u32, kind: u16) -> u32 {
        let key = self.at as u32 + 1;
        if let Some(slot) = self.tree.get_mut(self.at) {
            *slot = Node::new(key, parent, kind);
        }
        self.at += 1;
        key
    }

    /// Append a draw with a paint, which is the only property a draw can carry
    /// across this ring while there is no path encoding.
    fn draw(&mut self, parent: u32, tint: u16) -> u32 {
        let key = self.node(parent, kind::DRAW);
        if let Some(slot) = self.tree.get_mut(self.at - 1) {
            slot.paint = SetPaint {
                node: NO_NODE,
                red_x65535: tint,
                green_x65535: tint / 2,
                blue_x65535: u16::MAX - tint,
                alpha_x65535: u16::MAX,
                stroke_width_x65536: 0,
            };
        }
        key
    }

    /// Append a transform translated by `(tx, ty)` device pixels.
    fn translate(&mut self, parent: u32, tx_x65536: i64, ty_x65536: i64) -> u32 {
        let key = self.node(parent, kind::TRANSFORM);
        if let Some(slot) = self.tree.get_mut(self.at - 1) {
            slot.transform = SetTransform {
                node: NO_NODE,
                a_x65536: 65_536,
                b_x65536: 0,
                c_x65536: 0,
                d_x65536: 65_536,
                tx_x65536,
                ty_x65536,
            };
        }
        key
    }
}

/// One pixel in 16.16. Unit: device pixels, scaled by 65 536.
const PX: i64 = 65_536;

/// Write the whole scene into `tree`, with the playhead at `playhead_x65536`.
///
/// Immediate mode: every node every time, in pre-order, and the reconciler is
/// what decides that 994 of them did not change. A `tree` shorter than
/// [`NODES`] is written as far as it goes and [`Built::written`] still says how
/// many the scene has, so a caller cannot mistake a truncated tree for the
/// scene.
pub fn build(tree: &mut [Node], playhead_x65536: i64) -> Built {
    let mut out = Out { tree, at: 0 };
    let mut parts = [(0usize, 0usize); PARTS];
    let mut enter = |out: &Out<'_>, part: usize| parts[part].0 = out.at;
    // `leave` is written as a closure over the same array after `enter`'s last
    // use would be neater and is not possible; the ends are recorded below.
    let mut ends = [0usize; PARTS];

    // surface: one semantic root, which is the window, and its opaque ground.
    enter(&out, 0);
    let root = out.node(NO_NODE, kind::SEMANTIC);
    out.draw(root, 0x1000);
    ends[0] = out.at;

    // chrome_bar: a transform over sixteen icons, five titles, four separators,
    // and the eighteen semantic nodes the menu is to a reader who cannot see it.
    enter(&out, 1);
    let chrome = out.translate(root, 0, 0);
    for icon in 0..16u16 {
        out.draw(chrome, 0x2000 + icon);
    }
    for title in 0..5u16 {
        out.draw(chrome, 0x2100 + title);
    }
    for separator in 0..4u16 {
        out.draw(chrome, 0x2200 + separator);
    }
    for _ in 0..18 {
        out.node(chrome, kind::SEMANTIC);
    }
    ends[1] = out.at;

    // track_headers: a group, and eight rows of a ground, a mute control and its
    // button semantics, a level meter and a name.
    enter(&out, 2);
    let headers = out.node(root, kind::SEMANTIC);
    for row in 0..8u16 {
        let header = out.node(headers, kind::SEMANTIC);
        out.draw(header, 0x3000 + row);
        let mute = out.draw(header, 0x3100 + row);
        out.node(mute, kind::SEMANTIC);
        out.draw(header, 0x3200 + row);
        out.draw(header, 0x3300 + row);
    }
    ends[2] = out.at;

    // viewport and scroll: the clip every content region has, and the content
    // transform, which did not move this frame.
    enter(&out, 3);
    let viewport = out.node(root, kind::CLIP);
    ends[3] = out.at;
    enter(&out, 4);
    let scroll = out.translate(viewport, -320 * PX, 96 * PX);
    ends[4] = out.at;

    // ruler: two hundred and forty ticks and twenty-four labels.
    enter(&out, 5);
    for tick in 0..240u16 {
        out.draw(scroll, 0x4000 + tick);
    }
    for label in 0..24u16 {
        out.draw(scroll, 0x4100 + label);
    }
    ends[5] = out.at;

    // clip_bodies: eight tracks of twelve clips — a clip, its body, its stroke,
    // its name, and for the first eight of each track a waveform.
    enter(&out, 6);
    for track in 0..8i64 {
        let lane = out.translate(scroll, 0, (40 + 180 * track) * PX);
        for clip in 0..12u16 {
            let bounds = out.node(lane, kind::CLIP);
            out.draw(bounds, 0x5000 + clip);
            out.draw(bounds, 0x5100 + clip);
            out.draw(bounds, 0x5200 + clip);
            if clip < 8 {
                out.draw(bounds, 0x5300 + clip);
            }
        }
    }
    ends[6] = out.at;

    // selection: one translucent layer over the clip between 4.2 s and 6.8 s on
    // track 3, and the mark it draws.
    enter(&out, 7);
    let selection = out.node(scroll, kind::LAYER);
    out.draw(selection, 0x6000);
    ends[7] = out.at;

    // playhead: the transform that moves, a line and a head under it. Three
    // nodes, and the whole of what this frame dirties.
    enter(&out, 8);
    let playhead = out.translate(scroll, playhead_x65536, 0);
    out.draw(playhead, 0x7000);
    out.draw(playhead, 0x7001);
    ends[8] = out.at;

    // canvas_model, which is under the viewport and after the scroll's subtree
    // in pre-order: the canvas, its eight tracks, ninety-six clips, twelve
    // markers — semantics only, contributing nothing to the picture and walked
    // all the same.
    enter(&out, 11);
    let canvas = out.node(viewport, kind::SEMANTIC);
    for _ in 0..8 {
        let track = out.node(canvas, kind::SEMANTIC);
        for _ in 0..12 {
            out.node(track, kind::SEMANTIC);
        }
    }
    for _ in 0..12 {
        out.node(canvas, kind::SEMANTIC);
    }
    ends[11] = out.at;

    // inspector: a floating panel with a shadow under it and a material behind
    // its content, and a form of labels, fields and numbers.
    enter(&out, 9);
    let panel = out.translate(root, 2_240 * PX, 120 * PX);
    out.node(panel, kind::EFFECT);
    let material = out.node(panel, kind::LAYER);
    out.node(material, kind::EFFECT);
    for mark in 0..25u16 {
        out.draw(material, 0x8000 + mark);
    }
    for _ in 0..25 {
        out.node(material, kind::SEMANTIC);
    }
    ends[9] = out.at;

    // status_line: one line of text and what it says to a reader.
    enter(&out, 10);
    let status = out.node(root, kind::SEMANTIC);
    out.draw(status, 0x9000);
    ends[10] = out.at;

    for (part, end) in parts.iter_mut().zip(ends) {
        part.1 = end;
    }
    Built { parts, playhead, written: out.at }
}

/// The most deltas one node of this scene costs on the frame it is created.
///
/// A creation and one property: a draw carries a paint, a transform carries a
/// matrix, and nothing carries both or a path. The test holds the scene to it,
/// and [`build_frames`] divides the cap by it.
/// Unit: deltas per node.
pub const DELTAS_PER_NEW_NODE_MAX: usize = 2;

/// How many nodes the tree grows by per frame while the scene is being built,
/// under a cap of `cap` non-commit deltas per frame.
///
/// **The cold scene is not one frame and cannot be.** It costs 1 709 deltas —
/// 995 creations, 703 paints, 11 transforms — against a cap of fifty that the
/// compositor's manifest declares (RFC 0128) and a batch of sixty-four that
/// `f_scene::commit::DELTAS_MAX` bounds, so no compositor in this tree could
/// take it whole. What a client does instead is what an application loading a
/// window does: it presents the part of the tree it has, and grows it. Each
/// build frame is a prefix of the pre-order, which is a tree, and the next node
/// is always a later sibling of something already there — so the reconciler
/// emits creations and properties and never a move.
///
/// Zero for a cap too small to create one node with its property, which the
/// frame refuses rather than looping.
/// Unit: nodes per frame.
#[must_use]
pub const fn build_step(cap: usize) -> usize {
    cap / DELTAS_PER_NEW_NODE_MAX
}

/// How many build frames the scene takes at a step of `step` nodes.
/// Unit: frames.
#[must_use]
pub const fn build_frames(step: usize) -> usize {
    if step == 0 { 0 } else { NODES.div_ceil(step) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use f_abi::scene::Entry;

    /// The scene file itself. Read, not transcribed: every count below is taken
    /// out of these bytes, so a scene that changed is a red test here rather
    /// than a builder quietly describing the old one.
    const SCENE_TOML: &str = include_str!("../../../claims/0033-scene/scene.toml");

    /// One `[[part]]` as the file states it.
    #[derive(Clone, Copy, Default)]
    struct Declared<'a> {
        name: &'a str,
        parent: &'a str,
        kinds: &'a str,
        nodes: u64,
    }

    fn unquote(value: &str) -> &str {
        value.trim().trim_matches('"')
    }

    fn number(value: &str) -> u64 {
        let mut n = 0u64;
        for c in value.trim().chars() {
            match c {
                '0'..='9' => n = n * 10 + u64::from(c as u8 - b'0'),
                '_' => {}
                _ => break,
            }
        }
        n
    }

    /// The parts, the census rows by name, and the dirty part — a line reader,
    /// because the file is this repository's TOML subset and the three shapes
    /// read here are `key = "text"`, `key = number` and `key = [list]`.
    fn declared() -> ([Declared<'static>; PARTS], [(&'static str, u64); 16], &'static str) {
        let mut parts = [Declared::default(); PARTS];
        let mut census = [("", 0u64); 16];
        let mut dirty = "";
        let mut count = 0usize;
        let mut rows = 0usize;
        let mut section = "";
        for line in SCENE_TOML.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                section = line;
                if line == "[[part]]" {
                    count += 1;
                }
                continue;
            }
            let Some((key, value)) = line.split_once('=') else { continue };
            let key = key.trim();
            match section {
                "[[part]]" => {
                    let part = &mut parts[count - 1];
                    match key {
                        "part" => part.name = unquote(value),
                        "parent" => part.parent = unquote(value),
                        "kinds" => part.kinds = value.trim(),
                        "nodes" => part.nodes = number(value),
                        _ => {}
                    }
                }
                "[census]" if key != "notes" => {
                    census[rows] = (key, number(value));
                    rows += 1;
                }
                "[scene]" if key == "dirty_part" => dirty = unquote(value),
                _ => {}
            }
        }
        assert_eq!(count, PARTS, "scene.toml declares a different number of parts");
        (parts, census, dirty)
    }

    fn census_row(census: &[(&str, u64)], name: &str) -> u64 {
        census.iter().find(|(key, _)| *key == name).map(|&(_, v)| v).expect(name)
    }

    const KIND_NAMES: [&str; 6] = ["transform", "clip", "layer", "draw", "effect", "semantic"];
    const CENSUS_NAMES: [&str; 6] =
        ["transforms", "clips", "layers", "draws", "effects", "semantics"];
    const KINDS: [u16; 6] =
        [kind::TRANSFORM, kind::CLIP, kind::LAYER, kind::DRAW, kind::EFFECT, kind::SEMANTIC];

    fn scene(playhead: i64) -> ([Node; NODES], Built) {
        let mut tree = [Node::UNUSED; NODES];
        let built = build(&mut tree, playhead);
        (tree, built)
    }

    /// The tree is the scene: every part where the file puts it, with the node
    /// count the file gives it, containing exactly the kinds the file lists, and
    /// the whole summing to the census kind by kind. Counted off the **built
    /// tree**, not off [`SCENE`] — the table is checked against both, so a table
    /// that agreed with the file while the builder drifted is red.
    #[test]
    fn the_tree_is_the_scene_the_file_declares() {
        let (parts, census, _) = declared();
        let (tree, built) = scene(playhead_x65536(0));
        assert_eq!(built.written, NODES);
        assert_eq!(census_row(&census, "nodes"), NODES as u64);
        let mut totals = [0u64; 6];
        for (index, part) in parts.iter().enumerate() {
            let ours = SCENE[index];
            assert_eq!(ours.name, part.name, "part {index} is named differently");
            assert_eq!(ours.parent, part.parent, "{} hangs under a different part", part.name);
            let (start, end) = built.parts[index];
            let mut counted = [0u64; 6];
            for node in &tree[start..end] {
                let k = KINDS.iter().position(|&k| k == node.kind).expect("a kind of the six");
                counted[k] += 1;
            }
            assert_eq!((end - start) as u64, part.nodes, "{} has the wrong node count", part.name);
            for k in 0..6 {
                assert_eq!(counted[k], u64::from(ours.kinds[k]), "{} {}", part.name, KIND_NAMES[k]);
                let listed = part.kinds.lists(&format_kind(KIND_NAMES[k]));
                assert_eq!(
                    counted[k] != 0,
                    listed,
                    "{} and its kinds list: {}",
                    part.name,
                    KIND_NAMES[k]
                );
                totals[k] += counted[k];
            }
            // Every top-level node of the part hangs under a node of its parent
            // part, and every other node under a node of this part.
            let parent_range =
                SCENE.iter().position(|p| p.name == part.parent).map(|i| built.parts[i]);
            for node in &tree[start..end] {
                let at = node.parent as usize;
                let inside = at > start && at <= end;
                let under_parent = match parent_range {
                    Some((ps, pe)) => at > ps && at <= pe,
                    None => node.parent == NO_NODE,
                };
                assert!(inside || under_parent, "{}: node {} hangs outside", part.name, node.key);
            }
        }
        for k in 0..6 {
            assert_eq!(
                totals[k],
                census_row(&census, CENSUS_NAMES[k]),
                "census {}",
                CENSUS_NAMES[k]
            );
        }
    }

    fn format_kind(name: &str) -> FormattedKind<'_> {
        FormattedKind(name)
    }

    /// `"draw"` with its quotes, as a `kinds` list holds it, without a `String`.
    struct FormattedKind<'a>(&'a str);

    trait ContainsKind {
        fn lists(&self, kind: &FormattedKind<'_>) -> bool;
    }

    impl ContainsKind for &str {
        fn lists(&self, kind: &FormattedKind<'_>) -> bool {
            self.split(['[', ']', ',']).any(|item| unquote(item) == kind.0)
        }
    }

    /// The warm frame, through the real reconciler: the whole tree rebuilt with
    /// the playhead one step on emits **exactly one** delta, and it is the
    /// playhead's transform — and the subtree under that transform is the
    /// scene's `dirty_nodes`, read from the file. The attack this answers is a
    /// scene whose one moving node is the only node, where *one delta* would be
    /// true of any reconciler: here 994 nodes are rebuilt and compared.
    #[test]
    fn the_warm_frame_is_one_transform_over_the_dirty_part() {
        let (_, census, dirty) = declared();
        let (first, built) = scene(playhead_x65536(0));
        let (second, _) = scene(playhead_x65536(1));
        let mut reconciler = Reconciler::<NODES>::new();
        let mut deltas = Deltas::<64>::new();
        // Built the way the frame builds it, a prefix at a time under the cap:
        // the cold scene is 1 709 deltas and no buffer here holds it whole.
        let step = build_step(50);
        for frame in 0..build_frames(step) {
            let upto = ((frame + 1) * step).min(NODES);
            reconciler.frame(&first[..upto], &mut deltas).expect("a build frame reconciles");
        }
        reconciler.frame(&second, &mut deltas).expect("the warm frame reconciles");
        let emitted = deltas.as_slice();
        assert_eq!(emitted.len(), 1, "the warm frame emitted {emitted:?}");
        let Entry::SetTransform(set) = emitted[0] else { panic!("not a transform: {emitted:?}") };
        assert_eq!(set.node, built.playhead);
        assert_eq!(set.tx_x65536, playhead_x65536(1));

        let dirty_part = SCENE.iter().position(|p| p.name == dirty).expect("the dirty part");
        let (start, end) = built.parts[dirty_part];
        assert_eq!(built.playhead as usize, start + 1, "the playhead transform roots its part");
        let under =
            second.iter().filter(|n| n.key == built.playhead || n.parent == built.playhead).count();
        assert_eq!(under as u64, census_row(&census, "dirty_nodes"));
        assert_eq!((end - start) as u64, census_row(&census, "dirty_nodes"));

        // And the same tree a third time is nothing at all.
        reconciler.frame(&second, &mut deltas).expect("an unchanged frame reconciles");
        assert!(deltas.as_slice().is_empty());
    }

    /// The build: prefixes of the pre-order, each under the manifest's cap,
    /// together creating every node once and never moving or removing one.
    #[test]
    fn the_cold_scene_is_built_in_frames_under_the_cap() {
        let cap = 50;
        let step = build_step(cap);
        let (tree, _) = scene(playhead_x65536(0));
        let mut reconciler = Reconciler::<NODES>::new();
        let mut deltas = Deltas::<64>::new();
        let mut created = 0usize;
        let mut total = 0usize;
        for frame in 0..build_frames(step) {
            let upto = ((frame + 1) * step).min(NODES);
            reconciler.frame(&tree[..upto], &mut deltas).expect("a build frame reconciles");
            let emitted = deltas.as_slice();
            assert!(emitted.len() <= cap, "build frame {frame} is {} deltas", emitted.len());
            for entry in emitted {
                match entry {
                    Entry::CreateNode(create) => {
                        created += 1;
                        assert!(create.node as usize > frame * step, "a node created twice");
                    }
                    Entry::SetPaint(_) | Entry::SetTransform(_) => {}
                    other => panic!("a build frame emitted {other:?}"),
                }
            }
            total += emitted.len();
        }
        assert_eq!(created, NODES);
        // Eleven transforms and not twelve: the chrome bar sits at the origin,
        // and an identity is the value a new node already holds, so it costs
        // its creation and nothing else. That is the reconciler's *exactly the
        // differences* observed on a frame a node is born, and a count of 1 710
        // here would be a reconciler sending a property nobody changed.
        assert_eq!(total, 1_709, "995 creations, 703 paints and 11 transforms");
        assert_eq!(build_frames(step), 40);
    }

    /// No node costs more than [`DELTAS_PER_NEW_NODE_MAX`] on its first frame,
    /// which is what [`build_step`] divides by. Measured one node at a time.
    #[test]
    fn no_node_costs_more_than_the_step_assumes() {
        let (tree, _) = scene(playhead_x65536(0));
        let mut reconciler = Reconciler::<NODES>::new();
        let mut deltas = Deltas::<64>::new();
        for upto in 1..=NODES {
            reconciler.frame(&tree[..upto], &mut deltas).expect("one more node reconciles");
            assert!(deltas.as_slice().len() <= DELTAS_PER_NEW_NODE_MAX, "node {upto}");
        }
    }
}
