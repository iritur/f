// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The application, as something a projection can be pointed at.
//!
//! # Why a declaration is shipped rather than kept in a test
//!
//! `node.rs` built the settings panel as a `#[cfg(test)]` fixture, which was
//! right while the only question about it was *can the vocabulary hold it*. It
//! stopped being right the moment two tasks were written whose exits are about
//! **one unmodified application**: `E3-B06g` says the settings-panel tree
//! `interface/src/node.rs` already builds must reach pixels with no application
//! code beyond its declaration, and `E3-B06h` says one declaration must be
//! presented at two densities and two refresh rates from one unmodified
//! application. A fixture neither projection can reach is a fixture each of them
//! copies, and two copies of an application are two applications: the day
//! somebody adds a node here, the projection that copied it presents a tree
//! nobody declares any more and its exit goes on passing.
//!
//! So the declaration is public, it is the same value both projections are
//! handed, and *unmodified* becomes a property of the call rather than a claim
//! in a report.
//!
//! # What this is not
//!
//! It is not an application: nothing here runs, nothing holds a handle, and no
//! ring carries it. It is a **declaration** — the only artefact RFC 0077's
//! vocabulary asks an application for — and that is exactly what a projection
//! consumes. *What would reverse this:* a component that declares a tree over a
//! semantic channel, at which point the projections point at that component and
//! this module goes back to being a test's fixture, or goes away.
//!
//! # The rule this module keeps
//!
//! **Nothing here is added for a projection's benefit.** A projection that
//! cannot present what is declared says so — `f_semantic::remote::presentable`
//! is a named refusal for exactly that reason — and the repair is in the
//! projection or in the vocabulary, never here. A field added to this file
//! because a remote wanted it would make every *unmodified application* claim in
//! the tree false at once.

use crate::node::{
    CapRef, Constraints, Content, Flow, Node, NodeId, Quantity, Reading, Relation, Role, StateSet,
    Text, TokenName, TokenSet, Unit,
};

/// How many nodes the settings panel has.
/// Unit: nodes.
pub const SETTINGS_NODES: usize = 15;

/// The first hard interface: a settings panel.
///
/// Chosen because it is where *style is tokens, never values* and *layout is
/// constraints, never coordinates* are most tempting to break — a settings
/// panel is a column of labelled controls, and every toolkit that ever shipped
/// one grew a way to say *put this at 340 and make it blue*.
///
/// Fifteen nodes and no role outside the vocabulary. The volume is a
/// [`Role::Number`] with bounds and a step, so an agent can set it without
/// guessing what is settable; the mute toggle says what it disables with
/// [`Relation::Controls`]; every control says which label names it, so no
/// projection has to infer a name from proximity.
///
/// The nodes are in declaration order and every node's parent is before it,
/// which is not decoration: it is what a delta stream requires, what
/// [`Layout::declare`](crate::solve::Layout::declare) requires, and what paint
/// order in a scene graph means. A reordering here is a change to three things.
#[must_use]
pub fn settings_panel() -> [Node; SETTINGS_NODES] {
    const SURFACE: NodeId = NodeId::new(1);
    const RULE: NodeId = NodeId::new(2);
    const OUTPUT: NodeId = NodeId::new(10);
    const DEVICE_LABEL: NodeId = NodeId::new(11);
    const DEVICE: NodeId = NodeId::new(12);
    const SPEAKERS: NodeId = NodeId::new(13);
    const HEADPHONES: NodeId = NodeId::new(14);
    const VOLUME_LABEL: NodeId = NodeId::new(15);
    const VOLUME: NodeId = NodeId::new(16);
    const MUTE: NodeId = NodeId::new(17);
    const APPLIED: NodeId = NodeId::new(18);
    const INPUT: NodeId = NodeId::new(20);
    const NAME_LABEL: NodeId = NodeId::new(21);
    const NAME: NodeId = NodeId::new(22);
    const RESET: NodeId = NodeId::new(23);

    let percent = |value: i64| Quantity::whole(value, Unit::Ratio);

    [
        Node::new(SURFACE, NodeId::UNNAMED, Role::Surface)
            .with_content(text("Sound"))
            .with_layout(Constraints::flowing(Flow::Block))
            .with_style(styled(&["surface-1"])),
        Node::new(OUTPUT, SURFACE, Role::Group)
            .with_content(text("Output"))
            .with_layout(Constraints::flowing(Flow::Block).at_least(1200)),
        Node::new(DEVICE_LABEL, OUTPUT, Role::Label).with_content(text("Device")),
        related(
            Node::new(DEVICE, OUTPUT, Role::Choice)
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x0051))
                .with_layout(Constraints::flowing(Flow::Block)),
            &[Relation::LabelledBy(DEVICE_LABEL)],
        ),
        Node::new(SPEAKERS, DEVICE, Role::Item)
            .with_content(text("Speakers"))
            .with_state(StateSet::ENABLED.with(StateSet::SELECTED))
            .with_intent(CapRef::new(0x0052)),
        Node::new(HEADPHONES, DEVICE, Role::Item)
            .with_content(text("Headphones"))
            .with_state(StateSet::ENABLED)
            .with_intent(CapRef::new(0x0053)),
        Node::new(VOLUME_LABEL, OUTPUT, Role::Label).with_content(text("Volume")),
        related(
            Node::new(VOLUME, OUTPUT, Role::Number)
                .with_content(Content::Value(
                    Reading::bounded(percent(70), percent(0), percent(100)).stepped(percent(5)),
                ))
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x0054))
                .with_layout(Constraints::flowing(Flow::Inline).growing(1)),
            &[Relation::LabelledBy(VOLUME_LABEL)],
        ),
        related(
            Node::new(MUTE, OUTPUT, Role::Toggle)
                .with_content(text("Mute"))
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x0055)),
            &[Relation::Controls(VOLUME)],
        ),
        Node::new(APPLIED, OUTPUT, Role::Status)
            .with_content(text("Applied"))
            .with_style(styled(&["text.muted"])),
        Node::new(RULE, SURFACE, Role::Separator),
        Node::new(INPUT, SURFACE, Role::Group)
            .with_content(text("Input"))
            .with_layout(Constraints::flowing(Flow::Block)),
        Node::new(NAME_LABEL, INPUT, Role::Label).with_content(text("Microphone name")),
        related(
            Node::new(NAME, INPUT, Role::Entry)
                .with_content(text("built-in"))
                .with_state(StateSet::ENABLED.with(StateSet::INVALID))
                .with_intent(CapRef::new(0x0056))
                .with_layout(Constraints::flowing(Flow::Inline).at_least(800).growing(1))
                .with_style(styled(&["field.danger"])),
            &[Relation::LabelledBy(NAME_LABEL)],
        ),
        Node::new(RESET, INPUT, Role::Command)
            .with_content(text("Reset to defaults"))
            .with_state(StateSet::ENABLED)
            .with_intent(CapRef::new(0x0057))
            .with_style(styled(&["emphasis"])),
    ]
}

/// A label, or nothing said.
///
/// The error arm is unreachable — every string above is well inside
/// [`TEXT_MAX`](crate::node::TEXT_MAX) — and it answers rather than panicking
/// for [`Text::as_str`]'s reason: this module is compiled into whatever links
/// the vocabulary, and a fixture that panics is a fixture that can end somebody
/// else's component. A string too long declares nothing, which `check` sees and
/// a reader sees.
const fn text(value: &str) -> Content {
    match Text::new(value) {
        Ok(text) => Content::Text(text),
        Err(_) => Content::None,
    }
}

/// The tokens a node wears, skipping anything this vocabulary would refuse.
///
/// Refusals are dropped rather than raised for [`text`]'s reason. A dropped
/// token is visible where it matters: the node resolves to its role's ink
/// instead of the one named here.
fn styled(names: &[&str]) -> TokenSet {
    let mut set = TokenSet::EMPTY;
    for name in names {
        if let Ok(name) = TokenName::new(name)
            && let Ok(wider) = set.push(name)
        {
            set = wider;
        }
    }
    set
}

/// A node with its edges, dropping anything past
/// [`RELATIONS_MAX`](crate::node::RELATIONS_MAX) for [`text`]'s reason.
fn related(node: Node, edges: &[Relation]) -> Node {
    let mut built = node;
    for edge in edges {
        if let Ok(wider) = built.with_relation(*edge) {
            built = wider;
        }
    }
    built
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::{canvas_census, check};

    /// The declaration this crate ships is one this crate's own rules admit.
    ///
    /// The same assertion `node.rs` made when the fixture lived there, kept here
    /// rather than left behind: a public declaration that `check` refuses would
    /// be this crate shipping a tree it tells applications not to write.
    #[test]
    fn the_shipped_declaration_is_one_the_vocabulary_admits() {
        let panel = settings_panel();
        check(&panel).expect("a settings panel the vocabulary can hold");
        assert_eq!(canvas_census(&panel), Some((0, SETTINGS_NODES)));
    }

    /// Every node's parent is declared before it.
    ///
    /// Asserted rather than left to the eye, because three consumers depend on
    /// it — a delta stream, `Layout::declare`, and paint order — and none of
    /// them would report *the fixture was reordered*; they would each report
    /// something else.
    #[test]
    fn a_parent_is_always_declared_before_its_children() {
        let panel = settings_panel();
        for (at, node) in panel.iter().enumerate() {
            if !node.parent.is_named() {
                continue;
            }
            assert!(
                panel[..at].iter().any(|before| before.id == node.parent),
                "node {} is declared before its parent",
                node.id.value()
            );
        }
    }

    /// Every token this declaration wears is a name a theme resolves.
    ///
    /// The half of *style is tokens, never values* that a typo breaks. The other
    /// half — that there is no field a coordinate could go in — is held by the
    /// vocabulary itself and needs no assertion here.
    #[test]
    fn every_token_the_declaration_wears_is_a_name_the_theme_resolves() {
        for node in settings_panel() {
            for name in node.style.iter() {
                assert!(
                    crate::token::Token::from_name(name.as_str()).is_some(),
                    "{} wears {}, which no theme resolves",
                    node.role.name(),
                    name.as_str()
                );
            }
        }
    }
}
