// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Draw: what the display projection sets for a node, chosen by what the node
//! holds and never by what it is called.
//!
//! [`crate::emit`] decides a node's **kind** and says in its own header that it
//! emits no geometry — *nothing in this tree flattens a glyph run or a rectangle
//! into one yet*. This module is the text half of that geometry's input: for a
//! node the display projection puts marks down for, and whose content is text,
//! the paragraph [`f_text::paragraph`] sets. It is not a rasteriser and does not
//! produce a pixel; `E3-B02e`, `E3-B02h` and `E3-B03g` are where coverage comes
//! from, `E3-B06g` is where it reaches a screen, and RFC 0082's import is where
//! a glyph comes from. What it produces is what all of those will consume: which
//! scalars share a line, how wide each line is on the device's grid, and the
//! order each line is shown in. `E3-B03k`.
//!
//! # The rule: text is drawn because it is text
//!
//! `E3-B03k`'s exit forbids a *text-specific branch in either projection*, and
//! this module is where one would be most natural to write — `match node.role {
//! Role::Text => set it, … }`. There is none, and the absence is structural
//! rather than a matter of care:
//!
//! - **What is set is decided by [`Content`]**, by an exhaustive `match` with no
//!   wildcard. [`Content::Text`] is set, whatever role holds it: a
//!   [`Role::Command`](f_interface::node::Role) whose content is *Play* is set
//!   through exactly the lines a paragraph of prose is. The other three content
//!   kinds are named, not defaulted, so a fifth kind is `error[E0004]` here.
//! - **Whether anything is drawn is decided by the kind's family**, which is
//!   [`crate::emit::kind_of`]'s answer read through [`Kind::family`] — the rule
//!   that already decides where a paint goes. A node that puts no mark on the
//!   target draws none of its words either: a surface's title and a group's
//!   name are what a *reader* says about them, not ink. That is a per-role
//!   decision, and it is taken once, in `kind_of`, for all twenty-two roles
//!   alike; this module reads it and does not repeat it.
//!
//! So this file names no role, and `cargo xtask lint-projections` refuses one
//! here: the row for this file says it is role-free, which is stricter than the
//! rule for a file that holds a per-role table.
//!
//! # One declaration
//!
//! [`Display::of`] borrows the tree, exactly as `f_interface::reader::Reader::of`
//! does, and [`Display::tree`] hands the borrow back. A consumer that wants to
//! know whether a drawing and a reading came from one declaration compares the
//! two slices by address — `core::ptr::eq` — and the test that holds `E3-B03k`'s
//! exit does exactly that. *Equal* would not do: two declarations of one
//! application are equal until somebody edits one of them.
//!
//! # What is a parameter, and whose it is
//!
//! - **The measure**, in device pixels. `f_interface::solve` places a node
//!   along its parent's flow and solves no cross axis, so the width a paragraph
//!   is set in is not something the tree can yet answer; the compositor that
//!   holds the surface does. *Reversal:* a solve with a cross axis, at which
//!   point the measure is the node's solved extent converted by
//!   [`crate::emit::Surface`] and stops being an argument.
//! - **The scale and the advances**: the face's grid, the size it is set at and
//!   the shaper's answer per run. The shaper is RFC 0082's import and has not
//!   landed, and [`f_text::paragraph`] says why the seat is a parameter rather
//!   than a guess.
//! - **The base direction** is not a parameter: it is
//!   [`BaseDirection::FirstStrong`], because the vocabulary has no direction
//!   attribute and a text says which way it runs by its first strong scalar.
//!   *Reversal:* a direction in RFC 0077's vocabulary, which is an RFC.

use f_interface::node::{Content, Defect, Node, NodeId, TEXT_MAX, check, find};
use f_scene::kind::{Family, Kind};
use f_text::bidi::BaseDirection;
use f_text::metric::{Advance, NotAMetric, Px, Scale};
use f_text::paragraph::{SCALARS_MAX, Setting, Unset};

use crate::emit::kind_of;

// A node's text is at most `TEXT_MAX` bytes and so at most that many scalars;
// a setting holds `SCALARS_MAX`. `f_text` may not name `f_interface`, so the
// relation between the two crates' bounds is held here, where both are named,
// and a vocabulary that grew its text past the setting is a build failure
// rather than a paragraph refused at run time.
const _: () = assert!(TEXT_MAX <= SCALARS_MAX, "a declared text fits one setting");

/// A declared tree the display projection may draw from.
///
/// Admitted by the vocabulary's own [`check`], for `Reader::of`'s reason: a
/// drawing of a tree that is not one would be a third opinion about it.
#[derive(Clone, Copy, Debug)]
pub struct Display<'t> {
    tree: &'t [Node],
}

/// What drawing one node produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drawn<'t> {
    /// The node's text, set in the [`Setting`] the caller lent: `lines` lines.
    Set {
        /// The node, borrowed from the declaration.
        node: &'t Node,
        /// How many lines. Unit: lines.
        lines: usize,
    },
    /// The node's kind puts no mark on the target, so none of its words are
    /// ink. The same derivation `emit` makes for a paint.
    Unmarked {
        /// The node.
        node: &'t Node,
    },
    /// The node's content is not text: nothing, a reading, or media. A reading
    /// becomes words only once somebody chooses how to say a number, which is a
    /// projection's choice of language and not yet made for the display.
    Untextual {
        /// The node.
        node: &'t Node,
    },
}

/// Why a node could not be drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Undrawn {
    /// No node in the tree has this identity.
    Absent(NodeId),
    /// The node's text could not be set; carried whole.
    Unset(NodeId, Unset),
}

impl<'t> Display<'t> {
    /// Meet a tree, and be admitted or refused.
    ///
    /// # Errors
    ///
    /// The vocabulary's [`Defect`], whole.
    pub fn of(tree: &'t [Node]) -> Result<Self, Defect> {
        check(tree)?;
        Ok(Self { tree })
    }

    /// The declaration being drawn, as borrowed.
    #[must_use]
    pub const fn tree(&self) -> &'t [Node] {
        self.tree
    }

    /// Draw one node into `into`.
    ///
    /// Every node takes this path, in the same order: the kind's family first,
    /// then the content's kind. Neither step reads what the role is called.
    ///
    /// # Errors
    ///
    /// [`Undrawn::Absent`] for an identity the tree does not hold, and
    /// [`Undrawn::Unset`] for a text [`Setting::set`] refused.
    pub fn draw<M>(
        &self,
        id: NodeId,
        into: &mut Setting,
        measure: Px,
        scale: Scale,
        advance: M,
    ) -> Result<Drawn<'t>, Undrawn>
    where
        M: FnMut(&[char]) -> Result<Advance, NotAMetric>,
    {
        let node = find(self.tree, id).ok_or(Undrawn::Absent(id))?;
        if marks(kind_of(node.role)) {
            let text = match &node.content {
                Content::Text(text) => text,
                Content::None | Content::Value(_) | Content::Media(_) => {
                    return Ok(Drawn::Untextual { node });
                }
            };
            let lines = into
                .set(text.as_str(), BaseDirection::FirstStrong, scale, measure, advance)
                .map_err(|unset| Undrawn::Unset(id, unset))?
                .len();
            return Ok(Drawn::Set { node, lines });
        }
        Ok(Drawn::Unmarked { node })
    }
}

/// Does a node of this kind put marks on the target? `emit`'s rule for a paint,
/// read rather than restated.
fn marks(kind: Kind) -> bool {
    kind.family() == Family::Mark
}

#[cfg(test)]
mod tests {
    use super::*;
    use f_interface::canvas::Arrangement;
    use f_interface::example::settings_panel;
    use f_interface::node::{Role, Text};
    use f_interface::reader::{Part, Reader, Utterance};
    use f_interface::token::{Theme, resolve};

    const SURFACE: NodeId = NodeId::new(1);
    const OUTPUT: NodeId = NodeId::new(2);
    const VOLUME: NodeId = NodeId::new(3);
    const NOTE: NodeId = NodeId::new(4);
    const PLAY: NodeId = NodeId::new(5);

    /// The paragraph, as the author declared it. Sixty-five scalars, one word
    /// of them Hebrew so that one line has two directions on it.
    const NOTE_TEXT: &str = "Sound plays through the device named \u{05E9}\u{05DC}\u{05D5}\u{05DD} until another is chosen";

    fn text(value: &str) -> Content {
        Content::Text(Text::new(value).expect("fits"))
    }

    /// The one declaration: a window, a group, a label, a paragraph and a
    /// button whose content is text too.
    fn declared() -> [Node; 5] {
        [
            Node::new(SURFACE, NodeId::UNNAMED, Role::Surface).with_content(text("Settings")),
            Node::new(OUTPUT, SURFACE, Role::Group).with_content(text("Output")),
            Node::new(VOLUME, OUTPUT, Role::Label).with_content(text("Volume")),
            Node::new(NOTE, OUTPUT, Role::Text).with_content(text(NOTE_TEXT)),
            Node::new(PLAY, OUTPUT, Role::Command).with_content(text("Play")),
        ]
    }

    /// The seat, filled with a constant: half an em per scalar. Not a shaper;
    /// `f_text::paragraph`'s tests say what it is for.
    fn monospace(run: &[char]) -> Result<Advance, NotAMetric> {
        Advance::in_design_units(500 * i32::try_from(run.len()).unwrap_or(i32::MAX))
    }

    /// A 1000-unit grid at a 32-pixel em: sixteen pixels a scalar.
    fn scale() -> Scale {
        Scale::new(1000, 32 * 64).expect("a scale")
    }

    /// Twenty scalars of room.
    fn measure() -> Px {
        Px::new(320).expect("a width")
    }

    /// Does the setting hold exactly `want`, in logical order, from `from` to
    /// `to`?
    fn holds(setting: &Setting, from: usize, to: usize, want: &str) -> bool {
        want.chars().eq(setting.scalars()[from..to].iter().copied())
    }

    /// **`E3-B03k`'s exit, as far as this tree can hold it.** One declaration;
    /// the screen reader says it and the display projection sets it; both
    /// borrowed the same slice; the words set are the words said; the paragraph
    /// breaks where `f_text::line` offers and is shown in the order UAX #9
    /// gives each line; and nothing on either side asked what the role was
    /// called.
    #[test]
    fn one_declaration_is_said_and_set_and_the_two_agree() {
        let tree = declared();
        let reader = Reader::of(&tree, &[] as &[Arrangement]).expect("a tree");
        let display = Display::of(&tree).expect("a tree");
        assert!(
            core::ptr::eq(reader.nodes(), display.tree()),
            "the reading and the drawing were handed two declarations"
        );

        let mut heard = [Utterance::UNSAID; 16];
        let said = reader.read(&mut heard).expect("room");
        let mut setting = Setting::new();
        let mut set_nodes = 0;
        for line in &heard[..said] {
            if line.part == Part::Leaving {
                continue;
            }
            let drawn = display
                .draw(line.node, &mut setting, measure(), scale(), monospace)
                .expect("draws");
            let Drawn::Set { node, lines } = drawn else { continue };
            set_nodes += 1;
            // The words set are the words said, scalar for scalar, and both
            // are the node's own.
            let Content::Text(spoken) = line.content else { panic!("said text, set nothing") };
            assert!(
                spoken.as_str().chars().eq(setting.scalars().iter().copied()),
                "{:?} was said as {:?} and set as something else",
                node.id,
                spoken.as_str()
            );
            assert!(core::ptr::eq(node, &tree[tree.iter().position(|n| n.id == node.id).unwrap()]));
            assert_eq!(lines, setting.lines().len());
        }
        assert_eq!(
            set_nodes, 3,
            "the label, the paragraph and the button; not the window or the group"
        );

        // The spoken phrase for the two roles the task names.
        let label = heard[..said].iter().find(|l| l.node == VOLUME).expect("said");
        assert_eq!((label.phrasing.noun, label.content), ("label", text("Volume")));
        let note = heard[..said].iter().find(|l| l.node == NOTE).expect("said");
        assert_eq!((note.phrasing.noun, note.content), ("text", text(NOTE_TEXT)));
        // Each said once and whole, as the button holding text is: a
        // paragraph is not something a listener enters and leaves. RFC 0140's
        // lint cannot see this — the reader's text row set to `entered: true`
        // is a group's row and so a shared value — and this is where that
        // change goes red instead.
        for id in [VOLUME, NOTE, PLAY] {
            let parts: usize = heard[..said].iter().filter(|l| l.node == id).count();
            let line = heard[..said].iter().find(|l| l.node == id).expect("said");
            assert_eq!((parts, line.part), (1, Part::Whole), "{id:?} is said in parts");
        }
        // And painted as the button is, none of the three wearing a token. A
        // stronger statement than RFC 0140's second condition, which passes a
        // text row equal to *any* other role's — a status's muted ink included
        // — and so it is held here, where changing it is a visible decision
        // about how text looks rather than a row nobody reads.
        let (resolved, _report) = resolve(&Theme::DEFAULT);
        let painted = |id: NodeId| resolved.paint(find(&tree, id).expect("declared"));
        assert_eq!(painted(VOLUME), painted(PLAY), "the label is not inked as the button");
        assert_eq!(painted(NOTE), painted(PLAY), "the paragraph is not inked as the button");

        // The rendered paragraph: set again, and read line by line.
        let drawn = display.draw(NOTE, &mut setting, measure(), scale(), monospace).expect("draws");
        assert!(matches!(drawn, Drawn::Set { lines: 4, .. }), "{drawn:?}");
        let lines = setting.lines();
        let opportunities: [usize; 12] = {
            let mut out = [0; 12];
            for (k, o) in f_text::line::opportunities(setting.scalars()).enumerate() {
                out[k] = o.at;
            }
            out
        };
        // Greedy over twenty scalars of room, spaces hanging: 5+1+5+1+7 = 19
        // and `the` would make 23; 3+1+6+1+5 = 16 and the Hebrew word would
        // make 21; 4+1+5+1+7 = 18 and `is` would make 21; the rest is 9.
        let ends = [20, 37, 56, 65];
        for (line, end) in lines.iter().zip(ends) {
            assert_eq!(line.end, end, "a line of {lines:?} ended somewhere else");
            assert!(opportunities.contains(&line.end), "{end} is not an opportunity");
        }
        assert!(holds(&setting, 0, 20, "Sound plays through "));
        assert!(holds(&setting, 20, 37, "the device named "));
        assert!(holds(&setting, 37, 56, "\u{05E9}\u{05DC}\u{05D5}\u{05DD} until another "));
        assert!(holds(&setting, 56, 65, "is chosen"));
        assert_eq!(
            [lines[0].width.px(), lines[1].width.px(), lines[2].width.px(), lines[3].width.px()],
            [19 * 16, 16 * 16, 18 * 16, 9 * 16],
            "widths are the pen's, trailing spaces hanging"
        );
        // The third line is shown with the Hebrew reversed and nothing else:
        // N1 and N2 give both spaces beside it the paragraph's direction, and
        // L2 reverses the level-1 run alone.
        let shown = setting.visual(&lines[2]);
        assert_eq!(&shown[..5], &[40, 39, 38, 37, 41], "the Hebrew word, right to left");
        assert!(shown[4..].windows(2).all(|w| w[1] == w[0] + 1), "the Latin, as written");
    }

    /// One unmodified application: the shipped settings panel's labels reach a
    /// setting, and the longest breaks at the space, because ten scalars of
    /// room will not hold fifteen.
    #[test]
    fn the_shipped_panel_sets_its_labels_with_no_code_of_its_own() {
        let panel = settings_panel();
        let display = Display::of(&panel).expect("the panel is a tree");
        let mut setting = Setting::new();
        let narrow = Px::new(160).expect("a width");
        let mut labels = 0;
        for node in &panel {
            let drawn =
                display.draw(node.id, &mut setting, narrow, scale(), monospace).expect("draws");
            if let Drawn::Set { node, lines } = drawn
                && node.role == Role::Label
            {
                labels += 1;
                let words = if let Content::Text(t) = node.content { t } else { Text::EMPTY };
                if words.as_str() == "Microphone name" {
                    assert_eq!(lines, 2);
                    assert_eq!(setting.lines()[0].end, 11, "after `Microphone `");
                }
            }
        }
        assert_eq!(labels, 3, "Device, Volume and Microphone name");
    }

    /// A newline a declared text holds ends one line, after the LF of a CR LF
    /// and never between the two: `Text::new` admits both, and the reader
    /// says them as they are.
    #[test]
    fn a_declared_newline_ends_one_line() {
        for (words, first) in [("one\r\ntwo", 5), ("one\ntwo", 4)] {
            let tree = [
                Node::new(SURFACE, NodeId::UNNAMED, Role::Surface),
                Node::new(NOTE, SURFACE, Role::Text).with_content(text(words)),
            ];
            let display = Display::of(&tree).expect("a tree");
            let mut setting = Setting::new();
            let drawn = display.draw(NOTE, &mut setting, measure(), scale(), monospace);
            assert!(matches!(drawn, Ok(Drawn::Set { lines: 2, .. })), "{words:?}: {drawn:?}");
            assert_eq!(setting.lines()[0].end, first, "{words:?}");
        }
    }

    /// A content kind that is not text sets nothing, whatever the role, and a
    /// node that puts no mark on the target sets nothing, whatever its content.
    #[test]
    fn what_is_set_follows_the_content_and_the_kind_and_not_the_role() {
        let tree = [
            Node::new(SURFACE, NodeId::UNNAMED, Role::Surface).with_content(text("Settings")),
            Node::new(NOTE, SURFACE, Role::Text),
        ];
        let display = Display::of(&tree).expect("a tree");
        let mut setting = Setting::new();
        let window = display.draw(SURFACE, &mut setting, measure(), scale(), monospace);
        assert!(matches!(window, Ok(Drawn::Unmarked { .. })), "{window:?}");
        let empty = display.draw(NOTE, &mut setting, measure(), scale(), monospace);
        assert!(matches!(empty, Ok(Drawn::Untextual { .. })), "{empty:?}");
        let absent = display.draw(PLAY, &mut setting, measure(), scale(), monospace);
        assert_eq!(absent, Err(Undrawn::Absent(PLAY)));
    }
}
