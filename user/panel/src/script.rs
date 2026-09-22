// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The interface this component declares, as entries rather than as a picture.
//!
//! # Why this is not in `component.rs`
//!
//! Because both sides read it. The component submits these entries and the
//! frame checks what it applied against [`DECLARED`] — and a boot that compared
//! the component's report against a *copy* of the script would be comparing two
//! things that were edited together. It is the split `user/compositor` makes
//! between `tree` and `component`, one crate over, and it earns its keep the
//! same way: what is here compiles on a host and on both architectures, and the
//! entry point does not.
//!
//! # Why the roles are named and not numbered
//!
//! Every ordinal below comes out of [`Role`] through
//! `f_abi::semantic::Agreed::admit_role`, which asks `f_semantic::Interface`
//! whether the agreed vocabulary names it. So this file contains no integer that
//! means a role, and a role removed from `interface/src/node.rs` stops this
//! build rather than putting an ordinal nobody names on a wire. RFC 0083 calls
//! the alternative *a second decoder* and names it as the reversal to watch.
//!
//! # What the interface is, and why it is this small
//!
//! A surface with a group in it, holding a command and a label; the command is
//! enabled. Four nodes and one state, chosen so that every number the boot
//! prints can be checked by eye against this file — not so that the panel is
//! useful. It is deliberately the smallest declaration that has a parent, a
//! sibling order, a state and an intent in it, because those are the four things
//! `f_semantic::Tree` stores and a script that exercised three of them would
//! leave one unchecked.

use f_abi::semantic::{
    Agreed, Commit, DeclareNode, Entry, Handshake, NO_INTENT, NO_NODE, Refusal, SetState,
};
use f_interface::node::{Role, StateSet};
use f_semantic::Interface;

/// The surface everything hangs under. Unit: none — a node identifier.
pub const SURFACE: u64 = 1;
/// The group inside it. Unit: none — a node identifier.
pub const GROUP: u64 = 2;
/// The command the group holds. Unit: none — a node identifier.
pub const COMMAND: u64 = 3;
/// The label beside it. Unit: none — a node identifier.
pub const LABEL: u64 = 4;

/// What operating [`COMMAND`] does.
///
/// A capability reference in this component's own space, and RFC 0077's most
/// consequential line is why it is one rather than a callback: a system that
/// knows what a control *does* can describe it to a screen reader without the
/// author writing a label, check whether a caller may invoke it, and undo it.
///
/// This build declares the number and invokes nothing. `E3-B06j` is the task
/// where an agent reaches a node's intent as a capability call, and until then
/// what this proves is only that the field crosses and is stored.
/// Unit: none — a capability reference.
pub const INTENT: u32 = 1;

/// The frame token every commit in this script carries.
///
/// Non-zero, because `f_abi::semantic::Commit` refuses a frame nobody named: a
/// refusal has to be reportable against something. The bytes spell `panel`.
/// Unit: none — a frame identifier, not a quantity.
pub const FRAME_TOKEN: u64 = 0x0070_616E_656C;

/// How many entries this component submits. Unit: entries.
///
/// Counted from [`entries`]'s return type rather than written down, so that an
/// entry added to the script moves this number without anybody remembering.
pub const ENTRIES: usize = 7;

/// The nodes this script declares, with the role each is declared under.
///
/// **What the frame checks the tree against.** It is a list of what is declared
/// and not a list of what the tree should hold, and the difference matters:
/// nothing here says the frame accepted any of it. `kernel/src/semantic.rs`
/// walks this list against the tree it kept and is what turns the two into one
/// claim.
/// Unit: none — node identifiers and their roles.
pub const DECLARED: [(u64, Role); 4] = [
    (SURFACE, Role::Surface),
    (GROUP, Role::Group),
    (COMMAND, Role::Command),
    (LABEL, Role::Label),
];

/// The state [`COMMAND`] is left in, which the frame reads back out of the tree
/// after this component has ended.
///
/// `ENABLED` and nothing else. A node carrying an intent and not this state is
/// *not operable*, which `f_interface::node::StateSet::NONE` calls the safe
/// default in the only direction that matters — so a tree that lost this entry
/// would still be a well-formed tree describing a control nobody may press, and
/// the boot would have nothing to notice. That is why the state is checked by
/// value rather than counted.
pub const STATE: StateSet = StateSet::ENABLED;

/// The agreement this build offers and reaches with an identical peer.
///
/// # Errors
///
/// [`Refusal::VersionUnsupported`] where this build's own range does not overlap
/// itself,
/// which cannot happen and is returned rather than unwrapped because a component
/// has no unwinder: a `panic!` here is an instruction that stops the core with
/// nothing written anywhere, and `crate::routing::stopped` exists so that every
/// way this component can fail has a word for it.
pub fn agreement() -> Result<Agreed, Refusal> {
    Handshake::HERE.negotiate()
}

/// The script, in the order it is submitted.
///
/// The handshake first, and it has to be: `f_abi::semantic::Delta::decode`
/// refuses every other opcode with [`Refusal::NotNegotiated`] until one has been
/// believed, and `Session::accept` binds the refusal to the channel epoch rather
/// than to the frame. An application that opened with a declaration would not
/// get a second chance on that channel.
///
/// The commit last, and nothing after it: this component declares one frame.
///
/// # Errors
///
/// [`Refusal`] where this build's own vocabulary does not admit its own roles,
/// which is `interface/src/node.rs` and `semantic/src/vocabulary.rs` disagreeing
/// — a build failure wearing a run-time refusal, and reported as one because
/// there is nowhere here to report it otherwise.
pub fn entries() -> Result<[Entry; ENTRIES], Refusal> {
    let agreed = agreement()?;
    Ok([
        Entry::DeclareVocabulary(Handshake::HERE),
        declared(agreed, SURFACE, NO_NODE, NO_NODE, Role::Surface, NO_INTENT)?,
        declared(agreed, GROUP, SURFACE, NO_NODE, Role::Group, NO_INTENT)?,
        // The label is declared *before* the command and the command asks to sit
        // in front of it, so the tree's child order is not the order the entries
        // arrived. That is the one property `DeclareNode::before` exists for,
        // and a receiver that appended everything would pass every count in this
        // boot without it.
        declared(agreed, LABEL, GROUP, NO_NODE, Role::Label, NO_INTENT)?,
        declared(agreed, COMMAND, GROUP, LABEL, Role::Command, INTENT)?,
        Entry::SetState(SetState {
            node: COMMAND,
            state: agreed.admit_state(STATE.bits(), &Interface)?,
        }),
        Entry::Commit(Commit { frame_token: FRAME_TOKEN }),
    ])
}

/// One node declaration, with its role admitted rather than numbered.
///
/// The one place this crate turns a [`Role`] into an ordinal, and it goes
/// through `Agreed::admit_role` rather than around it: the value that reaches
/// the wire is one `f_semantic::Interface` said the agreed vocabulary names, and
/// there is no other way to build one.
///
/// # Errors
///
/// [`Refusal::Unnamed`] for a role this build declares and its own vocabulary
/// does not admit, which is two files in this workspace disagreeing.
fn declared(
    agreed: Agreed,
    node: u64,
    parent: u64,
    before: u64,
    role: Role,
    intent: u32,
) -> Result<Entry, Refusal> {
    let role = agreed.admit_role(role_ordinal(role), &Interface)?;
    Ok(Entry::DeclareNode(DeclareNode { node, parent, before, role, intent }))
}

/// A role's position in the vocabulary, as the wire's width.
///
/// A function rather than a cast at each call site, so that the one conversion
/// from `usize` to `u16` in this crate is in one place with its own name.
/// `Role::COUNT` is twenty-two and the cast cannot lose anything; it is written
/// as a `try_into` anyway, because a vocabulary that grew past sixty-five
/// thousand roles should fail loudly rather than silently, and RFC 0077's
/// closedness is a decision somebody could reverse.
/// Unit: none — an ordinal in the declaring list's own order.
fn role_ordinal(role: Role) -> u16 {
    u16::try_from(role.index()).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_script_is_a_handshake_then_edits_then_one_commit() {
        let entries = entries().expect("this build admits its own roles");
        assert!(matches!(entries[0], Entry::DeclareVocabulary(_)), "the handshake is not first");
        assert!(
            matches!(entries[ENTRIES - 1], Entry::Commit(_)),
            "the commit is not last, so this script declares a frame that never closes",
        );
        let commits = entries.iter().filter(|entry| matches!(entry, Entry::Commit(_))).count();
        assert_eq!(commits, 1, "one frame, and a second commit would be a second one");
    }

    #[test]
    fn every_node_the_script_declares_is_in_the_list_the_frame_checks() {
        // The two halves this file exists to keep together. A node added to
        // `entries` and forgotten in `DECLARED` is a node the boot would not
        // look for, which is exactly the shape of a check that passes while the
        // property is false.
        let entries = entries().expect("this build admits its own roles");
        let mut found = 0;
        for entry in entries {
            let Entry::DeclareNode(declare) = entry else { continue };
            let (_, role) = DECLARED
                .into_iter()
                .find(|(node, _)| *node == declare.node)
                .expect("a node is declared that the frame is never told to look for");
            assert_eq!(
                Role::from_index(declare.role.get() as usize),
                Some(role),
                "node {} crosses as a role the list does not give it",
                declare.node,
            );
            found += 1;
        }
        assert_eq!(found, DECLARED.len(), "the list names a node the script never declares");
    }

    #[test]
    fn the_order_the_entries_arrive_in_is_not_the_order_the_children_sit_in() {
        // The clause `DeclareNode::before` exists for. If this ever became true
        // the boot would still pass every count it takes and would have stopped
        // checking child order, so it is asserted about the script rather than
        // left to the run.
        let entries = entries().expect("this build admits its own roles");
        let mut arrival = [0u64; DECLARED.len()];
        let mut at = 0;
        for entry in entries {
            if let Entry::DeclareNode(declare) = entry {
                arrival[at] = declare.node;
                at += 1;
            }
        }
        assert_eq!(arrival, [SURFACE, GROUP, LABEL, COMMAND]);
        // And the command asks to sit in front of the label, so the group's
        // children end up the other way round.
        let inserting = entries.into_iter().any(|entry| {
            matches!(entry, Entry::DeclareNode(declare) if declare.node == COMMAND && declare.before == LABEL)
        });
        assert!(inserting, "nothing in this script inserts, so child order is arrival order");
    }
}
