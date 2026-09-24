// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Emit: a declared tree becomes scene deltas, and the compositor cannot tell
//! which of the two kinds of author wrote them.
//!
//! `E3-B06f`'s exit is an equality between two paths — *one scene fed both ways
//! produces byte-identical graphs* — and the reason the equality is worth
//! building is not tidiness. Part III of `docs/design/ring-scene-boot.html` ends
//! by asking for exactly one property of part II, and it asks for it so that
//! **this layer can be measured and if necessary abandoned without taking the
//! renderer down with it**. A compositor that could tell a projected delta from
//! an authored one would have a code path only the semantic layer reaches, and
//! deleting the semantic layer would then be a change to the compositor. So the
//! property this module is answerable to is: *nothing downstream of here has a
//! reason to know this module exists.*
//!
//! That is why there is no projection type holding state, no identifier this
//! stage mints, and no second entry point into `f_scene`. What this module
//! produces is `f_abi::scene::Delta` values — the same values that arrive from a
//! client that never heard of a [`Role`] — handed to a sink the caller chose.
//!
//! # Why this crate and not `interface/` or `scene/`
//!
//! `interface/Cargo.toml` declines `f-abi` so that the vocabulary stays a leaf
//! that can be deleted, and `scene/` may not take `f-interface` because a graph
//! that depended on the meaning layer would be a graph the meaning layer could
//! take down with it — which is the property above, inverted. The join has to
//! live in a crate that is neither, and this is already that crate: the crate
//! documentation says so for the tree and `E3-B06j` said so for the agent's
//! capability call. This is the third instance of one shape.
//!
//! # What a projection is allowed to choose
//!
//! RFC 0104 says a projection is the layer allowed to choose words, and what
//! survives it is the `match`: a decision per role, written out, with no
//! wildcard arm — because a wildcard is where a role nobody decided about goes
//! to be rendered as nothing. [`kind_of`] is this projection's decision and it
//! chooses a **kind**, not a word.
//!
//! Three things are derived rather than decided a second time, and that is
//! deliberate: the **paint** is emitted exactly where the chosen kind's family
//! is [`Family::Mark`], because a node that puts nothing on the target has
//! nothing to be painted; the **axis** a translation runs along is the parent's
//! [`Flow`], which `f_interface::solve` hands over rather than interpreting
//! (*which direction is the emit stage's to read*, says its module header); and
//! the **identity** is the author's own. A second per-role table would be a
//! second place for a twenty-third role to be missed.
//!
//! # The identifier crosses unchanged, and that is RFC 0117
//!
//! A scene node is a `u32` and a [`NodeId`] is a `u64`, so this stage either
//! renumbers or refuses. It refuses. An identifier this stage minted would be a
//! second name for every node, and the equality above is a statement about
//! *identifiers* as much as about bytes: a delta whose node numbers were
//! invented by the projection is a delta no author could have written, because
//! no author knows them. RFC 0117 is that decision and states what it costs — an
//! obligation on applications that RFC 0077 does not state — and its reversal
//! condition is the first author with a real reason to number past
//! [`NODE_MAX`].
//!
//! # What this stage does not emit, and why each absence is a decision
//!
//! - **No geometry.** A `SetPath` names a range in a channel's inline arena and
//!   the path encoding is `E3-B02c`'s; nothing in this tree flattens a glyph run
//!   or a rectangle into one yet. So a [`Kind::Draw`] here carries a paint and
//!   no path, and a [`Kind::Clip`] restricts nothing until it has one. That is
//!   the half of the display projection `E3-B06g` still owes, and it is named
//!   here rather than papered over with an empty path — an empty path is not *no
//!   clip*, it is a clip that encloses nothing, which is a black screen with an
//!   argument attached.
//! - **No `SetEffect`.** Nothing in the vocabulary declares a blur or a shadow.
//!   An effect is a cost a renderer declares and a degrade ladder spends
//!   (`f_scene::effect`, `f_scene::degrade`), not a meaning an author states,
//!   and a role that projected to one would be this layer inventing an
//!   appearance.
//! - **No [`Kind::Semantic`] node.** This is the interesting one, because that
//!   kind exists precisely so that *role, label, value, relations* travel in the
//!   scene tree in the same commit as the drawing — which section 07 calls the
//!   cheapest correct decision in the document. The wire has seven opcodes and
//!   **not one of them sets a role**: a semantic node emitted today would say
//!   *something meaningful is here* and be unable to say what, at the cost of
//!   doubling the graph. So none is emitted, and the reversal condition is an
//!   eighth opcode in RFC 0102's shape — at which point this module grows one
//!   more delta per node and [`crate::Registry`] stops being the only route by
//!   which a meaning reaches a reader.
//! - **No `Commit`.** A commit carries a deadline, a deadline is a clock
//!   reading, and RFC 0004 gives this layer no clock. The frame is closed by
//!   whoever holds the `f_env::Env`, and `f_scene::commit` is what makes that
//!   closure atomic — so a projection that refuses half way through leaves
//!   nothing published, without this module needing a rollback of its own.
//!
//! # A seventh kind does not break this file, and a twenty-third role does
//!
//! `f_scene::kind` promises that a seventh kind is a compile error in every
//! consumer that decides *per kind*. This module decides per **role**, so the
//! promise does not reach it, and saying so is better than implying a guarantee:
//! a seventh kind would be a kind no role projects to, which is already true of
//! two of the six. A twenty-third role is `error[E0004]` at [`kind_of`], and
//! `the_projection_decides_every_role_and_carries_no_wildcard` holds the half
//! the compiler cannot — that the arms are still one per role.
//!
//! # No clock, no randomness, no float, no allocator
//!
//! Every delta here is a pure function of the node, the placement and the
//! resolved theme handed in. The order is the declaration's. The one conversion
//! that happens here — tenths of a point to device pixels in 16.16 — is integer
//! arithmetic widened to `i64` where the product would not fit, with the scale
//! in every name; the other one a projection needs, sRGB to the linear light the
//! wire carries, deliberately does **not** happen here. It is
//! `f_interface::token::Paint::ink_linear_x65535`, because a second transfer
//! function is how a colour comes to pass the readability check and reach a
//! screen as something else, and `cargo xtask lint-token-pair` refuses the table
//! by name anywhere outside that module. No `Vec`: the deltas are handed to a
//! sink one at a time, so this stage holds no buffer and has no bound of its own
//! to exceed. RFC 0004.

use f_abi::NO_DEADLINE;
use f_abi::scene::{CreateNode, Delta, Entry, NO_NODE, SetPaint, SetTransform};
use f_interface::node::{Flow, Node, NodeId, Role};
use f_interface::solve::{Layout, Placed};
use f_interface::token::Resolved;
use f_scene::kind::{Family, Kind};

/// The largest identifier a node can carry and still be projected.
///
/// `u32::MAX`, because `f_abi::scene::CreateNode::node` is a `u32` and
/// `f_abi::scene::NO_NODE` is zero. A tree whose identifiers run past it is
/// refused with [`Refused::Unprojectable`] rather than renumbered — RFC 0117.
/// Unit: none — the largest node identifier, not a quantity.
pub const NODE_MAX: u64 = u32::MAX as u64;

/// A fully opaque node.
///
/// Every paint this stage emits carries it, and that is a decision rather than a
/// default: opacity is a value, the vocabulary carries tokens and not values,
/// and a projection that faded a node would be choosing an appearance nobody
/// declared. A node that should disappear is a node an author removes.
/// *What would reverse this:* a state in `StateSet` meaning *present and not
/// participating*, at which point the fade is declared and this stage reads it.
/// Unit: none — an opacity, scaled so that 65 535 is opaque.
const OPAQUE_X65535: u16 = 65_535;

/// What a transform's numbers are scaled by: 16.16 fixed point.
///
/// `f_abi::scene::SetTransform`'s own scale, named here because this module does
/// arithmetic in it rather than passing it through.
/// Unit: none — the scale of a 16.16 fixed-point value.
const FIXED_X65536: i64 = 65_536;

/// What a placement's numbers are scaled by: tenths of a point.
/// Unit: none — the scale of a `pt_x10` value.
const POINT_X10: i64 = 10;

/// What a display scale is scaled by: thousandths.
/// Unit: none — the scale of a `x1000` value.
const SCALE_X1000: i64 = 1_000;

/// The one thing about the display this stage needs and cannot derive.
///
/// # Why this is a parameter and not a metric
///
/// `Resolved` already carries the display's *density* — `Metric::Density`, in
/// thousandths — and it is already spent: the resolved em is the text size times
/// the density, so a point here is a point on this display. What nobody has
/// written down anywhere in this tree is how many device pixels one point is,
/// and that is a property of the panel rather than of the theme. A number
/// invented here would be a number with no owner.
///
/// So it is a parameter, supplied by whoever knows the panel — the compositor.
/// *What would reverse this:* the display's characteristics reaching this layer
/// as a type, which is `E3-B06d`'s own direction of travel; at that point this
/// struct is a field of that type and the parameter goes away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Surface {
    /// Device pixels per point, in thousandths. One thousand is one pixel per
    /// point; 1 333 is what a 96-pixel inch comes to against a 72-point one.
    /// Unit: device pixels per point, scaled by 1 000.
    pub px_per_pt_x1000: u32,
}

/// Why a declaration did not reach the graph.
///
/// Every variant that can name the node does, because a projection that refused
/// without saying which node it refused would leave an author reading their own
/// tree looking for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// `NodeId::UNNAMED` names nothing, and `f_abi::scene::NO_NODE` is the same
    /// value meaning the same thing one layer down. `node::check` refuses the
    /// identity for its own reason; this is where the two refusals meet.
    Unnamed,
    /// The identifier does not fit in a scene node. RFC 0117.
    Unprojectable(NodeId),
    /// The layout holds no placement for this node, so there is nothing to
    /// emit: a declaration that never reached `Layout::declare`, or a parent
    /// that never did.
    Unplaced(NodeId),
}

impl Refused {
    /// One line, for a report with no room for a match.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Unnamed => "a node that names nothing cannot be projected",
            Self::Unprojectable(_) => "the identifier does not fit in a scene node",
            Self::Unplaced(_) => "the layout holds no placement for this node",
        }
    }
}

/// What a projection handed over.
///
/// Every field is incremented by the statement that emits the thing it counts,
/// so there is no summary step for a count to drift in — `solve::Work`'s rule,
/// one stage up the same pipeline. The interesting one is [`Emitted::nodes`]: on
/// an incremental frame it is the size of the re-solved set, which
/// `Layout::nodes_read_in` answers independently by walking the slots, so the
/// two can be compared and neither is the other's source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Emitted {
    /// Nodes projected.
    /// Unit: nodes.
    pub nodes: u32,
    /// `CreateNode` deltas.
    /// Unit: deltas.
    pub creates: u32,
    /// `SetTransform` deltas.
    /// Unit: deltas.
    pub transforms: u32,
    /// `SetPaint` deltas.
    /// Unit: deltas.
    pub paints: u32,
}

impl Emitted {
    /// A projection that emitted nothing.
    pub const NOTHING: Self = Self { nodes: 0, creates: 0, transforms: 0, paints: 0 };

    /// How many deltas that was.
    ///
    /// Derived, rather than a fifth field a caller could find disagreeing with
    /// the other four.
    /// Unit: deltas.
    #[must_use]
    pub const fn deltas(&self) -> u32 {
        self.creates.saturating_add(self.transforms).saturating_add(self.paints)
    }
}

/// What kind of scene node a role projects to.
///
/// **The projection's one decision, and the whole of it.** Twenty-two arms, no
/// wildcard, so a twenty-third role is `error[E0004]` here and whoever adds it
/// is asked what it draws rather than discovering later that a wildcard said
/// *nothing*. RFC 0104 is the decision that this shape is what must survive.
///
/// # The four answers
///
/// - [`Kind::Layer`] for a [`Role::Surface`], and for nothing else. A surface is
///   the one node a projection presents on its own, which is the one place an
///   intermediate target can be needed for a subtree as a whole.
/// - [`Kind::Transform`] for the roles that place their children and put down no
///   marks of their own: the structural containers, and [`Role::Choice`], whose
///   body genuinely is its items — a radio group has no appearance its items do
///   not have. [`Role::Track`] is here for the same reason: a lane places what
///   is on it.
/// - [`Kind::Clip`] for a [`Role::Canvas`], and for nothing else. It is the one
///   role whose pixels the application draws, so it is the one role whose
///   subtree must not paint outside what it was given. RFC 0078 calls the canvas
///   the loophole that could eat the thesis from inside; a restriction is the
///   smallest thing that keeps the loophole inside its own extent.
/// - [`Kind::Draw`] for everything that is a mark: the content roles, the
///   controls that have a body a caller has to hit, the [`Role::Separator`]
///   whose whole content is its own edge, and the two placed things on an
///   arrangement.
///
/// A [`Kind::Draw`] with children is legal and is what a control is here: the
/// control's own body, with its label drawn over it. *What would reverse this:*
/// a control whose body and whose content need different transforms, at which
/// point a role projects to a subtree and this function answers one rather than
/// a kind.
#[must_use]
pub const fn kind_of(role: Role) -> Kind {
    match role {
        Role::Surface => Kind::Layer,
        Role::Group => Kind::Transform,
        Role::List => Kind::Transform,
        Role::Tree => Kind::Transform,
        Role::Item => Kind::Transform,
        Role::Table => Kind::Transform,
        Role::Row => Kind::Transform,
        Role::Cell => Kind::Transform,
        Role::Separator => Kind::Draw,
        Role::Label => Kind::Draw,
        Role::Text => Kind::Draw,
        Role::Image => Kind::Draw,
        Role::Status => Kind::Draw,
        Role::Command => Kind::Draw,
        Role::Toggle => Kind::Draw,
        Role::Entry => Kind::Draw,
        Role::Choice => Kind::Transform,
        Role::Number => Kind::Draw,
        Role::Canvas => Kind::Clip,
        Role::Track => Kind::Transform,
        Role::Clip => Kind::Draw,
        Role::Marker => Kind::Draw,
    }
}

/// The whole tree, as the deltas that build it.
///
/// One `CreateNode`, one `SetTransform` and — where the kind puts marks on the
/// target — one `SetPaint` per node, in declaration order, which is paint order:
/// every create names `NO_NODE` as its sibling, so a node is appended after the
/// ones the author declared before it.
///
/// The caller has already declared this tree into `layout` and solved it; this
/// stage writes nothing there. That separation is what makes the equality
/// testable at all — a projection that also declared would be a projection whose
/// output depended on a state it kept.
///
/// # Errors
///
/// [`Refused::Unnamed`] and [`Refused::Unprojectable`] for an identity that
/// cannot cross, [`Refused::Unplaced`] for a node the layout does not hold. The
/// sink may already have been handed deltas when one of these is returned, and
/// that is deliberate: a frame is atomic at its `Commit`, `f_scene::commit` is
/// what holds that, so a refusal here is a frame that never closes rather than a
/// half-applied graph. A rollback in this module would be a second statement of
/// the same rule.
pub fn project<S>(
    tree: &[Node],
    layout: &mut Layout,
    resolved: &Resolved,
    surface: Surface,
    sink: &mut S,
) -> Result<Emitted, Refused>
where
    S: FnMut(Delta),
{
    let mut emitted = Emitted::NOTHING;
    for node in tree {
        let scene = scene_id(node.id)?;
        let parent = if node.parent.is_named() { scene_id(node.parent)? } else { NO_NODE };
        let kind = kind_of(node.role);
        sink(around(Entry::CreateNode(CreateNode {
            node: scene,
            parent,
            before: NO_NODE,
            kind: kind.wire(),
        })));
        emitted.creates = emitted.creates.saturating_add(1);

        let (offset_pt_x10, _extent_pt_x10) =
            layout.placed_pt_x10(node.id).ok_or(Refused::Unplaced(node.id))?;
        // The parent's own offset and the axis it arranges on, in that order of
        // importance: a translation is the *difference*, because a solved offset
        // is a cursor over the whole chain above it and a scene transform
        // composes down the tree. `Layout::placed_pt_x10` says so at length.
        let (along, origin_pt_x10) = if node.parent.is_named() {
            let flow = layout.demand(node.parent).ok_or(Refused::Unplaced(node.parent))?.flow;
            let (origin, _) =
                layout.placed_pt_x10(node.parent).ok_or(Refused::Unplaced(node.parent))?;
            (flow, origin)
        } else {
            (Flow::Own, 0)
        };
        sink(translation(scene, offset_pt_x10, origin_pt_x10, along, surface));
        emitted.transforms = emitted.transforms.saturating_add(1);

        if kind.family() == Family::Mark {
            sink(mark(scene, resolved.paint(node).ink_linear_x65535()));
            emitted.paints = emitted.paints.saturating_add(1);
        }
        emitted.nodes = emitted.nodes.saturating_add(1);
    }
    Ok(emitted)
}

/// What moved, as the deltas that move it.
///
/// **The incremental half, and the one the exit is really about.** A projection
/// that re-emitted the whole tree would produce the same bytes for a frame in
/// which everything changed and would defeat the point of part II entirely, so
/// this path emits one `SetTransform` per node the last solve pass re-solved and
/// nothing else: no creates, because those nodes are already in the graph, and
/// no paints, because a theme that did not change did not change a colour.
///
/// It needs no tree and no `Resolved` to do it, which is the check on the claim:
/// a function that cannot see the declaration cannot have walked it.
/// `Layout::walk_resolved` hands over exactly the set the pass planned, so the
/// work is bounded by `Scope` — RFC 0113 — rather than by the tree.
///
/// A node whose offset came back to zero still gets its transform, and that is
/// why the transform is unconditional rather than skipped when it would be the
/// identity: a graph holds the last transform it was sent, so *say nothing when
/// there is nothing to say* would leave a node stranded at last frame's offset.
///
/// # Errors
///
/// [`Refused::Unprojectable`] for an identifier that cannot cross. The walk
/// still completes — it is the solver's own bounded walk and stopping it part
/// way would leave its counters describing something that did not happen — and
/// the first refusal is the one returned.
pub fn reproject<S>(layout: &mut Layout, surface: Surface, sink: &mut S) -> Result<Emitted, Refused>
where
    S: FnMut(Delta),
{
    let mut emitted = Emitted::NOTHING;
    let mut refused: Option<Refused> = None;
    let handed = layout.walk_resolved(|placed: Placed| {
        if refused.is_some() {
            return;
        }
        match scene_id(placed.id) {
            Ok(scene) => {
                sink(translation(
                    scene,
                    placed.offset_pt_x10,
                    placed.origin_pt_x10,
                    placed.along,
                    surface,
                ));
                emitted.transforms = emitted.transforms.saturating_add(1);
                emitted.nodes = emitted.nodes.saturating_add(1);
            }
            Err(why) => refused = Some(why),
        }
    });
    if let Some(why) = refused {
        return Err(why);
    }
    // The walk's own tally against this module's, for `solve`'s reason: two
    // counts produced by two pieces of code, and a disagreement is a red test
    // rather than a number nobody can check. They cannot differ once every node
    // is known to be projectable, which the line above has just established.
    debug_assert!(handed == emitted.nodes, "the walk and the projection counted different sets");
    Ok(emitted)
}

/// The identifier this node answers to in the graph: its own.
///
/// RFC 0117. The refusal is the whole content of the decision — a stage that
/// renumbered would be a stage every later stage had to ask.
fn scene_id(id: NodeId) -> Result<u32, Refused> {
    if !id.is_named() {
        return Err(Refused::Unnamed);
    }
    u32::try_from(id.value()).map_err(|_| Refused::Unprojectable(id))
}

/// A delta around a body, with the envelope that opcode accepts.
///
/// No deadline, because only a `Commit` reads one and this module emits none; no
/// flags, because `FLAGS_ACCEPTED` is about a submission this module does not
/// make; `user_data` zero, because correlation is the submitter's and a
/// projection that invented one would be inventing an identity the author did
/// not ask for. `payload_offset` is likewise zero: these are decoded deltas, and
/// where a payload sits in a channel's arena is settled by whoever writes it
/// there.
const fn around(body: Entry) -> Delta {
    Delta { user_data: 0, class: 0, deadline: NO_DEADLINE, payload_offset: 0, flags: 0, body }
}

/// Where a node sits, as the transform that puts it there.
///
/// A translation and nothing else: the matrix is the identity, because a solve
/// answers *where along one axis* and this layer has no scale, no rotation and
/// no skew to state. The axis is the parent's [`Flow`], handed over by the
/// solver, and the match over it is written out for [`kind_of`]'s reason.
///
/// **The translation is a difference and not the solver's number.** A solved
/// offset is a cursor: `Layout::share` starts each parent's at the parent's own
/// offset, so the number a node carries is the sum of every offset above it. A
/// `Kind::Transform` composes down the tree — `f_scene::kind` says a transform
/// replaces the node's own and that composition is the traversal's — so sending
/// that sum as a translation would place every nested subtree at its ancestors'
/// offsets twice. The subtraction is therefore the load-bearing statement in
/// this function, and it is also the only thing here that a fixture on a tree
/// whose parents all sit at zero cannot catch: `authored_second_frame` is that
/// fixture's opposite, and it is what found this in the first place.
///
/// **Inline runs towards increasing x**, which is an assumption about reading
/// direction that nothing in this tree has yet written down: the vocabulary
/// carries no writing direction and `Resolved` answers none. *What would reverse
/// this:* the first declaration whose script runs the other way, at which point
/// the direction arrives with the resolve stage and this function reads it
/// instead of assuming it.
fn translation(
    node: u32,
    offset_pt_x10: i32,
    origin_pt_x10: i32,
    along: Flow,
    surface: Surface,
) -> Delta {
    let px_x65536 = device_x65536(offset_pt_x10.saturating_sub(origin_pt_x10), surface);
    let (tx_x65536, ty_x65536) = match along {
        Flow::Inline | Flow::Wrap => (px_x65536, 0),
        Flow::Block => (0, px_x65536),
        // Nothing arranged this node — it is a root, or its parent took
        // arrangement back. Its offset is not the solver's to have computed, so
        // it is not this stage's to state, and the identity is the honest
        // answer. `canvas.rs`'s `Placement` is where the other one comes from.
        Flow::Own => (0, 0),
    };
    around(Entry::SetTransform(SetTransform {
        node,
        a_x65536: FIXED_X65536,
        b_x65536: 0,
        c_x65536: 0,
        d_x65536: FIXED_X65536,
        tx_x65536,
        ty_x65536,
    }))
}

/// Tenths of a point to device pixels in 16.16, on this surface.
///
/// Widened to `i64` before the multiplication, because an offset near the top of
/// the `i32` range times 65 536 is past it by five orders of magnitude.
/// Truncating rather than rounding: a half-pixel rule belongs to whatever snaps
/// marks to the pixel grid, and a projection that rounded first would hand the
/// rasteriser a number it then rounded again.
fn device_x65536(offset_pt_x10: i32, surface: Surface) -> i64 {
    let offset = i64::from(offset_pt_x10);
    let scale = i64::from(surface.px_per_pt_x1000);
    offset
        .saturating_mul(scale)
        .saturating_mul(FIXED_X65536)
        .saturating_div(POINT_X10.saturating_mul(SCALE_X1000))
}

/// What a mark is painted with.
///
/// The node's resolved ink, fully opaque, with no stroke. The three intensities
/// arrive already in the linear light the wire carries, because the conversion
/// from a theme's sRGB is `f_interface::token`'s and not this stage's —
/// `Paint::ink_linear_x65535` argues that at length, and `lint-token-pair` is
/// what makes it a rule rather than a preference.
///
/// The stroke is zero because a mark's thickness is its geometry's — a separator
/// is a thin filled shape and not a stroked line with no path — and zero is a
/// stated value in `f_abi::scene::SetPaint` rather than a sentinel. *What would
/// reverse this:* the first role whose mark is genuinely a stroke of a theme's
/// thickness, at which point [`kind_of`] answers a shape as well as a kind and
/// `Resolved::stroke_pt_x10` is what fills the field.
fn mark(node: u32, ink_x65535: (u16, u16, u16)) -> Delta {
    let (red_x65535, green_x65535, blue_x65535) = ink_x65535;
    around(Entry::SetPaint(SetPaint {
        node,
        red_x65535,
        green_x65535,
        blue_x65535,
        alpha_x65535: OPAQUE_X65535,
        stroke_width_x65536: 0,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::cell::Cell;

    use f_abi::scene::{RemoveNode, kind as wire};
    use f_interface::node::{Constraints, Role};
    use f_interface::solve::{Demand, Scope};
    use f_interface::token::{Theme, resolve};
    use f_scene::arena::Arena;
    use f_scene::dirty::Dirty;
    use f_scene::encode::{Encoder, encode};
    use f_scene::kind::ByKind;

    /// The axis a root is offered.
    const VIEWPORT_PT_X10: i32 = 6_000;
    /// The display the fixtures project onto: one device pixel per point.
    ///
    /// One, so that a reader checking a transform by hand is checking the
    /// solver's arithmetic and not this stage's. The scale is exercised on its
    /// own in `a_surface_scales_every_translation_and_nothing_else`.
    const FLAT: Surface = Surface { px_per_pt_x1000: 1_000 };

    /// A delta a buffer holds before anything has been put in it.
    ///
    /// Never read: `Stream::len` is what says how much of the array means
    /// anything. It is a removal of node one because every field of a `Delta`
    /// has to be something, and a removal is the body that cannot be confused
    /// with anything this module emits.
    const FILLER: Delta = Delta {
        user_data: 0,
        class: 0,
        deadline: NO_DEADLINE,
        payload_offset: 0,
        flags: 0,
        body: Entry::RemoveNode(RemoveNode { node: 1 }),
    };

    /// How many deltas a buffered frame may hold.
    const STREAM_MAX: usize = 64;

    /// A frame's deltas, held so that two of them can be compared.
    struct Stream {
        deltas: [Delta; STREAM_MAX],
        len: usize,
    }

    impl Stream {
        const EMPTY: Self = Self { deltas: [FILLER; STREAM_MAX], len: 0 };

        fn push(&mut self, delta: Delta) {
            assert!(
                self.len < STREAM_MAX,
                "the fixture wants a larger buffer, or a projection emitted more than a frame's worth"
            );
            self.deltas[self.len] = delta;
            self.len += 1;
        }

        fn each(&self) -> &[Delta] {
            &self.deltas[..self.len]
        }
    }

    /// A node with a layout, spelled once so the fixtures read as trees.
    fn node(id: u64, parent: u64, role: Role, layout: Constraints) -> Node {
        let mut node = Node::new(NodeId::new(id), NodeId::new(parent), role);
        node.layout = layout;
        node
    }

    /// How many nodes the panel has.
    const PANEL_NODES: usize = 6;

    /// A settings panel: a surface stacking a row, a rule and a paragraph.
    ///
    /// Two axes on purpose — the surface stacks in `Block` and the group runs in
    /// `Inline` — because a projection that put every translation on one axis
    /// would pass a fixture with one.
    fn panel() -> [Node; PANEL_NODES] {
        [
            node(1, 0, Role::Surface, Constraints::flowing(Flow::Block)),
            node(2, 1, Role::Group, Constraints::flowing(Flow::Inline).at_least(400)),
            node(3, 2, Role::Label, Constraints::LEAF.at_least(400)),
            node(4, 2, Role::Toggle, Constraints::LEAF.at_least(600)),
            node(5, 1, Role::Separator, Constraints::LEAF.at_least(100)),
            node(6, 1, Role::Text, Constraints::LEAF.at_least(1_000)),
        ]
    }

    /// Declare a whole tree into a layout, in the order it is written.
    fn declare(tree: &[Node], layout: &mut Layout, resolved: &Resolved) {
        for node in tree {
            layout
                .declare(node.id, node.parent, Demand::of(resolved, node))
                .expect("the fixture declares parents before children");
        }
    }

    /// One delta, through the wire and back.
    ///
    /// **What a compositor holds is what a decoder gave it**, so a projected
    /// delta that could not survive `f_abi::scene`'s own encoding would be a
    /// delta no author could have sent — which is the exit's property failing at
    /// a level a byte comparison of two graphs would never reach. Every fixture
    /// below crosses both streams through here.
    fn crossed(delta: &Delta) -> Delta {
        let (sqe, payload) = delta.encode();
        let back = Delta::decode(&sqe, &payload).expect("a projected delta is wire-legal");
        assert_eq!(back, *delta, "the wire round trip changed the delta");
        back
    }

    /// Apply a stream to a graph and encode the frame it closes.
    ///
    /// The commit is the caller's, here as everywhere: this stage emits none,
    /// so the token is chosen by the fixture and both sides of every comparison
    /// are handed the same one.
    fn graph(stream: &[Delta], frame: u64, arena: &mut Arena, into: &mut Encoder) {
        let mut dirty = Dirty::CLEAN;
        for delta in stream {
            dirty.apply(arena, &crossed(delta)).expect("the graph accepts the delta");
        }
        encode(into, &dirty.at_commit(frame), arena);
    }

    /// The frame the panel is committed under.
    ///
    /// Distinct in every byte that is not zero, so a header that wrote the token
    /// at the wrong offset cannot produce the same image — `f_scene::encode`'s
    /// own fixture makes the same choice for the same reason.
    const PANEL_FRAME: u64 = 0x0BAD_F00D_0000_0001;

    /// The panel's deltas **as an author would have sent them**.
    ///
    /// # What this fixture is worth, stated before it is relied on
    ///
    /// It is written by the same hand as the projection, so it is not two
    /// independent implementations and this test is not a proof that the
    /// projection is right. What it *is* is a statement of the projection's
    /// output in a form that shares nothing with the code that produces it:
    /// there is no `kind_of`, no `Kind`, no `LINEAR_X100000`, no [`Surface`] and
    /// no arithmetic below — only the wire records, with every number written
    /// out. A change to what a role projects to, to which axis a flow runs
    /// along, to the rounding of a colour or to the scale of a translation
    /// breaks this test, and each of those is a decision this module makes and
    /// could make differently by accident.
    ///
    /// The numbers are derivable by hand and the derivations are here so that a
    /// reader can check them rather than trust them. The em is 105 tenths of a
    /// point, so a node asking for 400 hundredths of an em is 420 wide; the
    /// group's two children are therefore at 0 and 420 along its inline axis,
    /// the group comes to 1 050, and the rule and the paragraph follow it down
    /// the surface's block axis at 1 050 and 1 155. A translation is tenths of a
    /// point times 65 536 over ten: 420 becomes 2 752 512, 1 050 becomes
    /// 6 881 280, 1 155 becomes 7 569 408. An ink of `0x1A1A1A` is 1 033
    /// hundred-thousandths of linear light, which is 677 of 65 535; the rule's
    /// `0x767676` is 18 116, which is 11 872.
    fn authored_panel() -> Stream {
        let mut authored = Stream::EMPTY;
        authored.push(hang(1, NO_NODE, wire::LAYER));
        authored.push(place(1, 0, 0));
        authored.push(hang(2, 1, wire::TRANSFORM));
        authored.push(place(2, 0, 0));
        authored.push(hang(3, 2, wire::DRAW));
        authored.push(place(3, 0, 0));
        authored.push(ink(3, 677));
        authored.push(hang(4, 2, wire::DRAW));
        authored.push(place(4, 2_752_512, 0));
        authored.push(ink(4, 677));
        authored.push(hang(5, 1, wire::DRAW));
        authored.push(place(5, 0, 6_881_280));
        authored.push(ink(5, 11_872));
        authored.push(hang(6, 1, wire::DRAW));
        authored.push(place(6, 0, 7_569_408));
        authored.push(ink(6, 677));
        authored
    }

    /// An authored create: appended after whatever is already under its parent.
    fn hang(node: u32, parent: u32, kind: u16) -> Delta {
        wrapped(Entry::CreateNode(CreateNode { node, parent, before: NO_NODE, kind }))
    }

    /// The envelope an author puts round a scene delta, **written out here rather
    /// than borrowed from [`around`]**.
    ///
    /// A mutation found this: `around` carries five fields the encoding never
    /// sees — `user_data`, `class`, `deadline`, `flags`, `payload_offset` — so a
    /// fixture that called it would agree with any value the projection chose for
    /// them, and a delta carrying a correlation token no author asked for would
    /// pass every test in this module. The decoder catches two of the five
    /// (`crossed` is where), and this catches the rest.
    fn wrapped(body: Entry) -> Delta {
        Delta { user_data: 0, class: 0, deadline: 0, payload_offset: 0, flags: 0, body }
    }

    /// An authored translation, written as the wire record it is.
    fn place(node: u32, tx_x65536: i64, ty_x65536: i64) -> Delta {
        wrapped(Entry::SetTransform(SetTransform {
            node,
            a_x65536: 65_536,
            b_x65536: 0,
            c_x65536: 0,
            d_x65536: 65_536,
            tx_x65536,
            ty_x65536,
        }))
    }

    /// An authored paint: one grey, opaque, filled.
    fn ink(node: u32, level_x65535: u16) -> Delta {
        wrapped(Entry::SetPaint(SetPaint {
            node,
            red_x65535: level_x65535,
            green_x65535: level_x65535,
            blue_x65535: level_x65535,
            alpha_x65535: 65_535,
            stroke_width_x65536: 0,
        }))
    }

    /// **The exit's first half.** One scene fed both ways is the same graph,
    /// byte for byte.
    ///
    /// Two graphs, two encoders, one frame token, and the comparison is
    /// `bytes() == bytes()` — not a field-by-field walk of two arenas by a
    /// function this diff also wrote. The arena is not directly comparable (it
    /// is neither `PartialEq` nor readable as bytes without `unsafe`, which this
    /// crate forbids) and it does not need to be: the encoding **is** the
    /// canonical form of a frame, it is what stage 2 of the ladder receives, and
    /// `f_scene::encode`'s whole design is that it is a function of the values
    /// and of nothing about the machine. A difference the encoding cannot see is
    /// a difference no renderer can see either.
    ///
    /// The deltas are compared as well as the bytes, and the order matters: if
    /// the streams differ the failure names the delta, and if only the bytes
    /// differ the failure is in the graph rather than in the projection. One
    /// assertion would have made every failure look like the same failure.
    #[test]
    fn an_authored_frame_and_a_projected_frame_are_the_same_graph() {
        let (resolved, report) = resolve(&Theme::DEFAULT);
        assert!(report.is_clean(), "the default theme resolves clean: {report:?}");
        let tree = panel();
        let mut layout = Layout::new();
        declare(&tree, &mut layout, &resolved);
        layout.solve(VIEWPORT_PT_X10, Scope::UNBOUNDED).expect("the panel declares a boundary");

        let mut projected = Stream::EMPTY;
        let emitted = project(&tree, &mut layout, &resolved, FLAT, &mut |delta| {
            projected.push(delta);
        })
        .expect("the panel projects");
        assert_eq!(
            emitted,
            Emitted { nodes: 6, creates: 6, transforms: 6, paints: 4 },
            "six nodes, and a paint on each of the four that put marks down"
        );
        assert_eq!(emitted.deltas(), 16);

        let authored = authored_panel();
        assert_eq!(authored.len, projected.len, "the two streams are not the same length");
        for (at, (mine, theirs)) in projected.each().iter().zip(authored.each()).enumerate() {
            assert_eq!(mine, theirs, "delta {at} differs");
        }

        let mut from_projection = Arena::EMPTY;
        let mut projected_image = Encoder::EMPTY;
        graph(projected.each(), PANEL_FRAME, &mut from_projection, &mut projected_image);
        let mut from_author = Arena::EMPTY;
        let mut authored_image = Encoder::EMPTY;
        graph(authored.each(), PANEL_FRAME, &mut from_author, &mut authored_image);

        assert_eq!(from_projection.live(), PANEL_NODES);
        assert_eq!(from_author.live(), PANEL_NODES);
        assert_eq!(
            projected_image.bytes(),
            authored_image.bytes(),
            "the compositor can tell an authored delta from a projected one"
        );
    }

    /// How long the panel's encoding is.
    const PANEL_IMAGE_BYTES: usize = 432;

    /// The panel's frame, as bytes.
    ///
    /// **The third witness, and it is here because the other two share a
    /// hand.** The fixture above and the projection can be edited into agreement
    /// with each other; neither of them can be edited into agreement with this,
    /// because this is not records, it is the image `f_scene::encode` writes —
    /// the header, the widths, the order of the three optional records and the
    /// little-endian byte order of every number in them. `encode.rs` pins one of
    /// these for a scene an author built; this pins one for a scene a
    /// *declaration* built, which is the artifact `E3-B06g` will put on a
    /// screen.
    ///
    /// Read it as: `SCNE`, version 1, no flags, the frame token, six nodes, 432
    /// bytes; then six records of node, depth, kind, present bits and whichever
    /// of the three property records the bits claim.
    #[rustfmt::skip]
    const PANEL_IMAGE: [u8; PANEL_IMAGE_BYTES] = [
        0x53, 0x43, 0x4E, 0x45, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
        0x0D, 0xF0, 0xAD, 0x0B, 0x06, 0x00, 0x00, 0x00, 0xB0, 0x01, 0x00, 0x00,
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x00, 0x01, 0x00,
        0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x02, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00,
        0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x03, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x04, 0x00, 0x05, 0x00,
        0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0xA5, 0x02, 0xA5, 0x02, 0xA5, 0x02, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x00,
        0x04, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x04, 0x00, 0x05, 0x00,
        0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x2A, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0xA5, 0x02, 0xA5, 0x02, 0xA5, 0x02, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x00,
        0x05, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x04, 0x00, 0x05, 0x00,
        0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x69, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x60, 0x2E, 0x60, 0x2E, 0x60, 0x2E, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x00,
        0x06, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x04, 0x00, 0x05, 0x00,
        0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x73, 0x00, 0x00, 0x00, 0x00, 0x00,
        0xA5, 0x02, 0xA5, 0x02, 0xA5, 0x02, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x00,
    ];

    /// The projected panel's frame is that image.
    #[test]
    fn the_projected_panel_encodes_to_this_byte_image() {
        let (resolved, _report) = resolve(&Theme::DEFAULT);
        let tree = panel();
        let mut layout = Layout::new();
        declare(&tree, &mut layout, &resolved);
        layout.solve(VIEWPORT_PT_X10, Scope::UNBOUNDED).expect("no cascade");
        let mut stream = Stream::EMPTY;
        project(&tree, &mut layout, &resolved, FLAT, &mut |delta| stream.push(delta))
            .expect("the panel projects");
        let mut arena = Arena::EMPTY;
        let mut image = Encoder::EMPTY;
        graph(stream.each(), PANEL_FRAME, &mut arena, &mut image);
        assert_eq!(image.len(), PANEL_IMAGE_BYTES);
        assert_eq!(image.bytes(), &PANEL_IMAGE[..], "the projected frame's bytes moved");
    }

    /// How many sections the big fixture has.
    ///
    /// Sixty of eleven nodes plus a surface is 661, which is as close to
    /// `f_scene::arena::NODES_MAX` as a fixture can get and stay a tree: the
    /// graph holds a thousand and twenty-four nodes, and a projection that is
    /// one node per declaration cannot show incrementality on a tree that does
    /// not nearly fill it. `E3-B06e`'s own fixture is ten thousand nodes because
    /// nothing downstream of it had a bound; this one is bounded by the graph
    /// the deltas are going into, and that is the honest ceiling for this stage.
    /// Unit: sections.
    const SECTIONS: u64 = 60;
    /// Leaves under each section.
    /// Unit: leaves.
    const PER_SECTION: u64 = 10;
    /// The surface, plus a section and its leaves, sixty times.
    /// Unit: nodes.
    const BIG_NODES: usize = 1 + (SECTIONS * (1 + PER_SECTION)) as usize;
    /// One section and its leaves: what a change inside one re-solves.
    /// Unit: nodes.
    const SUBTREE_NODES: u32 = 1 + PER_SECTION as u32;
    /// A section's extent, definite in both directions, which is what makes it a
    /// solve boundary — RFC 0113.
    /// Unit: em_x100.
    const SECTION_EM_X100: u16 = 10_000;
    /// What a leaf asks for, so that ten of them come to exactly a section.
    /// Unit: em_x100.
    const LEAF_EM_X100: u16 = 1_000;
    /// What the one changed leaf asks for instead.
    /// Unit: em_x100.
    const SHRUNK_EM_X100: u16 = 500;
    /// The axis the big fixture's surface is offered.
    /// Unit: pt_x10.
    const BIG_VIEWPORT_PT_X10: i32 = 700_000;
    /// Which section the change is in, and which leaf of it.
    ///
    /// The seventh and the third, rather than the first of either: a projection
    /// that emitted the first subtree it found would pass a fixture that changed
    /// the first one.
    const CHANGED_SECTION: u64 = 7;
    /// Which leaf of that section changes.
    const CHANGED_LEAF: u64 = 2;
    /// How much room the second frame's image is given here.
    ///
    /// Eleven nodes of at most `NODE_MAX_BYTES` plus a header, rounded up. It is
    /// a test's buffer and not a bound on anything: the encoder's own bound is
    /// `ENCODING_MAX`, and this exists so that the frame under comparison can be
    /// held in a kilobyte rather than in a second encoder.
    /// Unit: bytes.
    const SECOND_FRAME_MAX: usize = 1_024;

    /// The frame the whole tree arrives in.
    const BIG_FRAME_ONE: u64 = 0x0BAD_F00D_0000_0002;
    /// The frame the one change arrives in.
    const BIG_FRAME_TWO: u64 = 0x0BAD_F00D_0000_0003;

    /// A section's identity. Disjoint from the leaves' range, so a mistyped
    /// fixture collides rather than aliasing quietly.
    fn section_id(at: u64) -> u64 {
        1_000 + at
    }

    /// A leaf's identity.
    fn leaf_id(at: u64, which: u64) -> u64 {
        10_000 + at * 100 + which
    }

    /// One leaf, asking for what it is told to ask for.
    fn leaf_node(at: u64, which: u64, want_em_x100: u16) -> Node {
        node(
            leaf_id(at, which),
            section_id(at),
            Role::Text,
            Constraints::LEAF.at_least(want_em_x100),
        )
    }

    /// Sixty definite sections of ten leaves, under one surface.
    fn big() -> [Node; BIG_NODES] {
        let mut tree = [node(1, 0, Role::Surface, Constraints::flowing(Flow::Block)); BIG_NODES];
        let mut at = 1;
        for section in 0..SECTIONS {
            tree[at] = node(
                section_id(section),
                1,
                Role::Group,
                Constraints::flowing(Flow::Block)
                    .at_least(SECTION_EM_X100)
                    .at_most(SECTION_EM_X100),
            );
            at += 1;
            for which in 0..PER_SECTION {
                tree[at] = leaf_node(section, which, LEAF_EM_X100);
                at += 1;
            }
        }
        assert_eq!(at, BIG_NODES, "the fixture built a different number of nodes than it declares");
        tree
    }

    /// The eleven translations **an author would have sent** for the second
    /// frame.
    ///
    /// The derivation, so that a reader can check it: the em is 105 tenths of a
    /// point, so a leaf asking for a thousand hundredths of an em is 1 050 wide
    /// and a section of ten of them is 10 500. Section seven therefore sits at
    /// 73 500 down the surface's block axis, and that number does not move —
    /// which is the whole of RFC 0113: a definite extent is a boundary, so
    /// nothing above it is re-solved. Inside it the third leaf drops to 525, so
    /// the leaves are at 0, 1 050, 2 100, 2 625, 3 675, 4 725, 5 775, 6 825,
    /// 7 875 and 8 925. A translation is tenths of a point times 65 536 over ten.
    ///
    /// The section comes first because the walk is preorder, and the order is
    /// load-bearing rather than incidental: the graph's dirty set coalesces a
    /// mark that encloses another, so the first delta of this stream is what
    /// makes the frame *one* subtree of eleven nodes instead of eleven marks.
    fn authored_second_frame() -> Stream {
        let mut authored = Stream::EMPTY;
        authored.push(place(1_007, 0, 481_689_600));
        authored.push(place(10_700, 0, 0));
        authored.push(place(10_701, 0, 6_881_280));
        authored.push(place(10_702, 0, 13_762_560));
        authored.push(place(10_703, 0, 17_203_200));
        authored.push(place(10_704, 0, 24_084_480));
        authored.push(place(10_705, 0, 30_965_760));
        authored.push(place(10_706, 0, 37_847_040));
        authored.push(place(10_707, 0, 44_728_320));
        authored.push(place(10_708, 0, 51_609_600));
        authored.push(place(10_709, 0, 58_490_880));
        authored
    }

    /// **The exit's second half, and the one that stops the first being cheap.**
    /// A frame that changed one leaf carries eleven nodes of six hundred and
    /// sixty-one, and it is byte-identical to the frame an author would have
    /// sent for the same change.
    ///
    /// A projection that re-emitted the whole tree would satisfy the equality
    /// above and defeat part II entirely, so the two clauses are held together
    /// here: the same comparison, on a frame whose content is a *difference*.
    /// Three numbers say the work was bounded and none of them is maintained by
    /// this module — `Work::nodes` is the solver's, `Layout::nodes_read_in` is a
    /// walk of the slots the solve path never performs, and `Encoder::nodes` is
    /// the count of records the encoder actually wrote.
    #[test]
    fn an_incremental_frame_carries_only_the_subtree_that_moved() {
        let (resolved, _report) = resolve(&Theme::DEFAULT);
        let tree = big();
        let mut layout = Layout::new();
        declare(&tree, &mut layout, &resolved);
        let whole = layout.solve(BIG_VIEWPORT_PT_X10, Scope::UNBOUNDED).expect("no cascade");
        assert_eq!(whole.nodes, BIG_NODES as u32, "the first pass is the whole tree");
        assert_eq!(whole.overflows, 0, "ten leaves come to exactly one section");

        // One graph and one encoder, reused, and the frame under test copied out
        // into a small buffer instead of a second 82 KiB encoder. That is not
        // tidiness: two arenas, three encoders and a layout is 1.3 MiB of stack
        // and the test thread has less than that. **The comparison survives the
        // saving**, because a `SetTransform` replaces rather than composes —
        // `f_scene::kind::Transform` says so — so applying the authored frame
        // over the projected one leaves the graph holding the authored numbers,
        // and the second encoding differs from the first exactly when the two
        // frames differ. A projected delta that was wrong would be overwritten
        // by a right authored one and the images would part; equal images mean
        // equal records.
        let mut arena = Arena::EMPTY;
        let mut marks = Dirty::CLEAN;
        let mut image = Encoder::EMPTY;
        let built = project(&tree, &mut layout, &resolved, FLAT, &mut |delta| {
            marks.apply(&mut arena, &crossed(&delta)).expect("the graph accepts the delta");
        })
        .expect("the tree projects");
        assert_eq!(built.nodes, BIG_NODES as u32);
        assert_eq!(built.creates, BIG_NODES as u32);
        assert_eq!(
            built.paints,
            (SECTIONS * PER_SECTION) as u32,
            "a leaf is a mark, a section is not"
        );
        assert_eq!(arena.live(), BIG_NODES);
        encode(&mut image, &marks.at_commit(BIG_FRAME_ONE), &arena);
        assert_eq!(image.nodes(), BIG_NODES as u32, "the first frame is the whole tree");
        let whole_frame_bytes = image.len();

        // One leaf says it needs half as much.
        let changed = leaf_node(CHANGED_SECTION, CHANGED_LEAF, SHRUNK_EM_X100);
        let said = layout
            .declare(changed.id, changed.parent, Demand::of(&resolved, &changed))
            .expect("a leaf may say again what it needs");
        assert!(said.changed && !said.inserted, "a re-declaration is not an insertion: {said:?}");
        let work = layout.solve(BIG_VIEWPORT_PT_X10, Scope::UNBOUNDED).expect("no cascade");
        assert_eq!(work.subtrees, 1, "a definite section is where the cascade stops");
        assert_eq!(work.nodes, SUBTREE_NODES);

        // Counted before it is buffered, and the reason is a mutation: a
        // projection that re-emitted the whole tree overflows the fixture's
        // buffer, and *the fixture wants a larger buffer* is the wrong sentence
        // to send a reader looking. Counting first makes the number the failure.
        // It also says something worth saying — the plan is read and not
        // consumed, so asking twice answers twice.
        let counted = Cell::new(0u32);
        let sized = reproject(&mut layout, FLAT, &mut |_delta| {
            counted.set(counted.get().saturating_add(1));
        })
        .expect("the re-solved set projects");
        assert_eq!(
            counted.get(),
            SUBTREE_NODES,
            "the second frame carries more nodes than the change reached"
        );

        let mut moved = Stream::EMPTY;
        let again = reproject(&mut layout, FLAT, &mut |delta| moved.push(delta))
            .expect("the re-solved set projects");
        assert_eq!(again, sized, "the plan was consumed by being walked");
        assert_eq!(
            again,
            Emitted { nodes: SUBTREE_NODES, creates: 0, transforms: SUBTREE_NODES, paints: 0 },
            "a second frame creates nothing and paints nothing"
        );
        assert_eq!(again.nodes, work.nodes, "the projection and the solver disagree about the set");
        assert_eq!(
            again.nodes,
            layout.nodes_read_in(layout.pass()),
            "the projection and a walk of the slots disagree about the set"
        );

        let authored = authored_second_frame();
        assert_eq!(authored.len, moved.len, "the two second frames are not the same length");
        for (at, (delta, theirs)) in moved.each().iter().zip(authored.each()).enumerate() {
            assert_eq!(delta, theirs, "delta {at} of the second frame differs");
        }

        let mut projected_image = [0u8; SECOND_FRAME_MAX];
        let mut marks = Dirty::CLEAN;
        let mut image = Encoder::EMPTY;
        for delta in moved.each() {
            marks.apply(&mut arena, &crossed(delta)).expect("the graph accepts it");
        }
        encode(&mut image, &marks.at_commit(BIG_FRAME_TWO), &arena);
        assert_eq!(
            image.nodes(),
            SUBTREE_NODES,
            "the second frame carries more nodes than the change reached"
        );
        let second_frame_bytes = image.len();
        assert!(second_frame_bytes <= SECOND_FRAME_MAX, "{second_frame_bytes} bytes");
        projected_image[..second_frame_bytes].copy_from_slice(image.bytes());
        assert!(
            second_frame_bytes * 20 < whole_frame_bytes,
            "a frame of eleven nodes should be a small fraction of a frame of 661: {} against {}",
            second_frame_bytes,
            whole_frame_bytes
        );

        let mut marks = Dirty::CLEAN;
        let mut image = Encoder::EMPTY;
        for delta in authored.each() {
            marks.apply(&mut arena, &crossed(delta)).expect("the graph accepts it");
        }
        encode(&mut image, &marks.at_commit(BIG_FRAME_TWO), &arena);
        assert_eq!(
            image.bytes(),
            &projected_image[..second_frame_bytes],
            "the compositor can tell an authored change from a projected one"
        );
    }

    /// This file, for the two properties the compiler cannot state.
    ///
    /// `interface/src/reader.rs` holds the same two about its own projection and
    /// this is deliberately the same device: a `match` with no wildcard is what
    /// RFC 0104 says must survive, and *a wildcard has not been added* is not
    /// something a type can carry.
    const SOURCE: &str = include_str!("emit.rs");

    /// The body of [`kind_of`]'s match, as text.
    ///
    /// The first `match role {` in the file is that function's, and everything up
    /// to the first line that closes a block at four spaces is its arms.
    fn kind_arms() -> &'static str {
        let after = SOURCE.split_once("match role {").expect("the projection matches on Role").1;
        after.split_once("\n    }\n}").expect("the match closes the function").0
    }

    /// Every one of the twenty-two roles is decided, and a twenty-third would be
    /// a compile error rather than a silence.
    ///
    /// The compile error is the compiler's: [`kind_of`] is a `match` over a closed
    /// enum with no wildcard, so `error[E0004]` is what a twenty-third variant
    /// produces here. What this test holds is the two things the compiler cannot:
    /// that the arms are still one per role rather than a wildcard somebody added
    /// in a hurry, and that no arm has been collapsed into a catch-all.
    #[test]
    fn the_projection_decides_every_role_and_carries_no_wildcard() {
        let arms = kind_arms();
        assert_eq!(
            arms.matches("Role::").count(),
            Role::COUNT,
            "one arm per role, and the count is read from the vocabulary rather than written here"
        );
        assert!(!arms.contains("_ =>"), "a wildcard arm is where a role nobody decided about goes");
        assert!(arms.contains("Role::Surface =>"), "the first role of the vocabulary");
        assert!(arms.contains("Role::Marker =>"), "the last role of the vocabulary");
    }

    /// What the twenty-two decisions come to, per kind.
    ///
    /// **Written out row by row, which is the point.** `f_scene::kind` argues it:
    /// an exhaustive match demands an arm and not an arm that *says* anything, so
    /// a table written as a literal is the guard a match cannot be — this one is
    /// the wrong length the day there is a seventh kind, in this crate, with this
    /// census in front of whoever added it. The two zeroes are decisions and are
    /// argued in the module header: an effect is a cost a renderer declares, and a
    /// semantic node cannot yet carry the role it exists to carry.
    #[rustfmt::skip]
    const CENSUS: ByKind<u16> = ByKind::new([
        // Transform: the seven structural containers that place children, plus a
        // choice and a track.
        9,
        // Clip: the canvas, and nothing else.
        1,
        // Layer: the surface, and nothing else.
        1,
        // Draw: the four content roles, the five controls with a body, the
        // separator and the two placed things on an arrangement.
        11,
        // Effect: no role declares one.
        0,
        // Semantic: no opcode carries a role, so no node is emitted to hold one.
        0,
    ]);

    /// Every role projects to a kind the wire knows, and the census is that one.
    #[test]
    fn every_role_projects_to_a_kind_the_wire_knows() {
        let mut found = ByKind::new([0u16; Kind::COUNT]);
        let mut total = 0;
        for role in Role::all() {
            let kind = kind_of(role);
            assert!(
                wire::known(kind.wire()),
                "{} projects to a kind this wire does not carry",
                role.name()
            );
            found[kind] = found[kind].saturating_add(1);
            total += 1;
        }
        assert_eq!(total, Role::COUNT, "the vocabulary is closed and this walked all of it");
        for kind in Kind::ALL {
            assert_eq!(found[kind], CENSUS[kind], "the census of {} moved", kind.name());
        }
    }

    /// A projected node carries a paint exactly when its kind puts marks down.
    ///
    /// Read back out of the **graph** rather than counted out of the stream: the
    /// arena is what a renderer traverses, and *this node has a paint* is a
    /// question it can answer. A projection that painted a transform would be
    /// giving a colour to a node that cannot put it anywhere, and one that left a
    /// mark unpainted would be drawing in whatever the last frame left.
    #[test]
    fn a_paint_reaches_exactly_the_nodes_that_put_marks_down() {
        let (resolved, _report) = resolve(&Theme::DEFAULT);
        let tree = panel();
        let mut layout = Layout::new();
        declare(&tree, &mut layout, &resolved);
        layout.solve(VIEWPORT_PT_X10, Scope::UNBOUNDED).expect("no cascade");
        let mut stream = Stream::EMPTY;
        project(&tree, &mut layout, &resolved, FLAT, &mut |delta| stream.push(delta))
            .expect("the panel projects");
        let mut arena = Arena::EMPTY;
        let mut image = Encoder::EMPTY;
        graph(stream.each(), PANEL_FRAME, &mut arena, &mut image);

        let mut marks = 0;
        for node in &tree {
            let scene = u32::try_from(node.id.value()).expect("the fixture numbers small");
            let kind = arena.kind_of(scene).expect("the graph holds every projected node");
            let painted = arena.paint_of(scene).expect("the graph holds it").is_some();
            assert_eq!(
                painted,
                kind.family() == Family::Mark,
                "{} projected to {} and is {}painted",
                node.role.name(),
                kind.name(),
                if painted { "" } else { "not " }
            );
            assert!(
                arena.path_of(scene).expect("the graph holds it").is_none(),
                "this stage has no geometry to send and must not pretend otherwise"
            );
            marks += u32::from(painted);
        }
        assert_eq!(marks, 4, "the label, the toggle, the rule and the paragraph");
    }

    /// An identifier that cannot cross is refused rather than renumbered.
    ///
    /// **RFC 0117's whole content**, and the three refusals are three different
    /// facts: nothing is named, the name does not fit, or the name is fine and the
    /// layout has never heard of it. A projection that collapsed them would send
    /// whoever hit one to the wrong file.
    #[test]
    fn an_identifier_that_cannot_cross_is_refused() {
        let (resolved, _report) = resolve(&Theme::DEFAULT);
        let mut layout = Layout::new();
        let sent = Cell::new(0u32);
        let mut count = |_delta: Delta| sent.set(sent.get().saturating_add(1));

        let unnamed = [node(0, 0, Role::Surface, Constraints::flowing(Flow::Block))];
        assert_eq!(
            project(&unnamed, &mut layout, &resolved, FLAT, &mut count),
            Err(Refused::Unnamed)
        );

        let past = NODE_MAX + 1;
        let huge = [node(past, 0, Role::Surface, Constraints::flowing(Flow::Block))];
        assert_eq!(
            project(&huge, &mut layout, &resolved, FLAT, &mut count),
            Err(Refused::Unprojectable(NodeId::new(past)))
        );
        assert_eq!(sent.get(), 0, "a refusal on an identity happens before anything is emitted");

        // The identity is fine and the layout has never been told about it.
        let stranger = [node(9, 0, Role::Surface, Constraints::flowing(Flow::Block))];
        assert_eq!(
            project(&stranger, &mut layout, &resolved, FLAT, &mut count),
            Err(Refused::Unplaced(NodeId::new(9)))
        );
        assert_eq!(sent.get(), 1, "the create went out before the placement was missed");
        assert!(!Refused::Unnamed.message().is_empty());
    }

    /// A pass that refused leaves nothing to re-emit.
    ///
    /// The half of `Layout::walk_resolved` that is about *not* emitting. A
    /// [`Cascade`] writes no placement and keeps the mark owed, so a projection
    /// that walked last frame's plan would send a frame of numbers nothing
    /// computed — and it would be a plausible frame, which is what makes it worth
    /// a test rather than a comment.
    ///
    /// [`Cascade`]: f_interface::solve::Cascade
    #[test]
    fn a_refused_pass_leaves_nothing_to_re_emit() {
        let (resolved, _report) = resolve(&Theme::DEFAULT);
        let tree = panel();
        let mut layout = Layout::new();
        declare(&tree, &mut layout, &resolved);
        let narrow = Scope { nodes_max: 3 };
        let refused = layout.solve(VIEWPORT_PT_X10, narrow).expect_err("six nodes is past three");
        assert_eq!(refused.nodes, PANEL_NODES as u32);
        assert_eq!(refused.scope_nodes, 3);

        let sent = Cell::new(0u32);
        let bump = |_delta: Delta| sent.set(sent.get().saturating_add(1));
        let emitted = reproject(&mut layout, FLAT, &mut bump.clone()).expect("nothing to do");
        assert_eq!(emitted, Emitted::NOTHING, "a refused pass left a plan to walk");
        assert_eq!(sent.get(), 0);

        // And the mark is still owed, so the frame after it is the one that
        // carries the work — which is the other half of `Cascade`'s promise.
        let done = layout.solve(VIEWPORT_PT_X10, Scope::UNBOUNDED).expect("no cascade");
        assert_eq!(done.nodes, PANEL_NODES as u32);
        let again = reproject(&mut layout, FLAT, &mut bump.clone()).expect("re-solved");
        assert_eq!(again.transforms, PANEL_NODES as u32);
        assert_eq!(sent.get(), PANEL_NODES as u32);

        // A pass with nothing marked plans nothing, so a frame that changed
        // nothing carries nothing.
        let idle = layout.solve(VIEWPORT_PT_X10, Scope::UNBOUNDED).expect("nothing marked");
        assert_eq!(idle, f_interface::solve::Work::NOTHING);
        let quiet = reproject(&mut layout, FLAT, &mut bump.clone()).expect("nothing to do");
        assert_eq!(quiet, Emitted::NOTHING, "a frame that changed nothing emitted something");
    }

    /// The surface scales every translation and touches nothing else.
    ///
    /// Two densities over one declaration, which is the shape `E3-B06h` asks of
    /// the remote projection and is worth having here as well: what a display
    /// changes is where marks land, and it is not what they are or what colour
    /// they are.
    #[test]
    fn a_surface_scales_every_translation_and_nothing_else() {
        let (resolved, _report) = resolve(&Theme::DEFAULT);
        let tree = panel();
        let mut layout = Layout::new();
        declare(&tree, &mut layout, &resolved);
        layout.solve(VIEWPORT_PT_X10, Scope::UNBOUNDED).expect("no cascade");

        let mut flat = Stream::EMPTY;
        project(&tree, &mut layout, &resolved, FLAT, &mut |delta| flat.push(delta))
            .expect("projects");
        let mut dense = Stream::EMPTY;
        let twice = Surface { px_per_pt_x1000: 2_000 };
        project(&tree, &mut layout, &resolved, twice, &mut |delta| dense.push(delta))
            .expect("projects");

        assert_eq!(flat.len, dense.len);
        for (at, (one, two)) in flat.each().iter().zip(dense.each()).enumerate() {
            match (one.body, two.body) {
                (Entry::SetTransform(one), Entry::SetTransform(two)) => {
                    assert_eq!(two.tx_x65536, one.tx_x65536 * 2, "transform {at}");
                    assert_eq!(two.ty_x65536, one.ty_x65536 * 2, "transform {at}");
                    assert_eq!(two.a_x65536, one.a_x65536, "a density is not a scale matrix");
                    assert_eq!(two.d_x65536, one.d_x65536, "a density is not a scale matrix");
                }
                _ => assert_eq!(one, two, "delta {at} is not a transform and must not move"),
            }
        }
    }
}
