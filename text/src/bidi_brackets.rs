// SPDX-License-Identifier: (Apache-2.0 OR MIT) AND Unicode-3.0
//
// Generated from Unicode's data by `cargo xtask unicode`. Do not edit: a change
// here is a change to `xtask/src/imported.rs`, regenerated and reviewed.
//
// Upstream-File: third_party/unicode/BidiBrackets.txt
// Upstream-URL: https://www.unicode.org/Public/17.0.0/ucd/BidiBrackets.txt
// Upstream-SHA-256: dadbaf38a0d0246e5b805bf8725cb81b7c621f93d030595635f5ba2c2f179428
// Unicode-Version: 17.0.0
// Regenerate: cargo xtask unicode
//
// Two licences, because what follows is Unicode's bracket pairs re-spelled
// as Rust. `third_party/unicode/LICENSE` is Unicode's notice, and
// `LICENSING.md` says what a redistributor of a binary owes, since a binary
// carries no header. RFC 0114.

//! `Bidi_Paired_Bracket` and `Bidi_Paired_Bracket_Type`, from Unicode 17.0.0.
//!
//! Every code point whose type is `Open` or `Close`, ascending, with the bracket
//! it pairs with. A code point not listed has type `None`, which is the upstream
//! file's own statement: it lists only the other two. [`crate::property`] reads it.

/// The Unicode version this table was generated from.
pub const UNICODE_VERSION: &str = "17.0.0";

/// Which side of a pair a bracket is on: `Bidi_Paired_Bracket_Type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BracketType {
    /// `o` in the upstream file.
    Open,
    /// `c` in the upstream file.
    Close,
}

/// Each bracket, the bracket it pairs with, and its type, ascending by the first.
#[rustfmt::skip]
pub const BIDI_BRACKETS: &[(char, char, BracketType)] = &[
    ('\u{0028}', '\u{0029}', BracketType::Open),
    ('\u{0029}', '\u{0028}', BracketType::Close),
    ('\u{005B}', '\u{005D}', BracketType::Open),
    ('\u{005D}', '\u{005B}', BracketType::Close),
    ('\u{007B}', '\u{007D}', BracketType::Open),
    ('\u{007D}', '\u{007B}', BracketType::Close),
    ('\u{0F3A}', '\u{0F3B}', BracketType::Open),
    ('\u{0F3B}', '\u{0F3A}', BracketType::Close),
    ('\u{0F3C}', '\u{0F3D}', BracketType::Open),
    ('\u{0F3D}', '\u{0F3C}', BracketType::Close),
    ('\u{169B}', '\u{169C}', BracketType::Open),
    ('\u{169C}', '\u{169B}', BracketType::Close),
    ('\u{2045}', '\u{2046}', BracketType::Open),
    ('\u{2046}', '\u{2045}', BracketType::Close),
    ('\u{207D}', '\u{207E}', BracketType::Open),
    ('\u{207E}', '\u{207D}', BracketType::Close),
    ('\u{208D}', '\u{208E}', BracketType::Open),
    ('\u{208E}', '\u{208D}', BracketType::Close),
    ('\u{2308}', '\u{2309}', BracketType::Open),
    ('\u{2309}', '\u{2308}', BracketType::Close),
    ('\u{230A}', '\u{230B}', BracketType::Open),
    ('\u{230B}', '\u{230A}', BracketType::Close),
    ('\u{2329}', '\u{232A}', BracketType::Open),
    ('\u{232A}', '\u{2329}', BracketType::Close),
    ('\u{2768}', '\u{2769}', BracketType::Open),
    ('\u{2769}', '\u{2768}', BracketType::Close),
    ('\u{276A}', '\u{276B}', BracketType::Open),
    ('\u{276B}', '\u{276A}', BracketType::Close),
    ('\u{276C}', '\u{276D}', BracketType::Open),
    ('\u{276D}', '\u{276C}', BracketType::Close),
    ('\u{276E}', '\u{276F}', BracketType::Open),
    ('\u{276F}', '\u{276E}', BracketType::Close),
    ('\u{2770}', '\u{2771}', BracketType::Open),
    ('\u{2771}', '\u{2770}', BracketType::Close),
    ('\u{2772}', '\u{2773}', BracketType::Open),
    ('\u{2773}', '\u{2772}', BracketType::Close),
    ('\u{2774}', '\u{2775}', BracketType::Open),
    ('\u{2775}', '\u{2774}', BracketType::Close),
    ('\u{27C5}', '\u{27C6}', BracketType::Open),
    ('\u{27C6}', '\u{27C5}', BracketType::Close),
    ('\u{27E6}', '\u{27E7}', BracketType::Open),
    ('\u{27E7}', '\u{27E6}', BracketType::Close),
    ('\u{27E8}', '\u{27E9}', BracketType::Open),
    ('\u{27E9}', '\u{27E8}', BracketType::Close),
    ('\u{27EA}', '\u{27EB}', BracketType::Open),
    ('\u{27EB}', '\u{27EA}', BracketType::Close),
    ('\u{27EC}', '\u{27ED}', BracketType::Open),
    ('\u{27ED}', '\u{27EC}', BracketType::Close),
    ('\u{27EE}', '\u{27EF}', BracketType::Open),
    ('\u{27EF}', '\u{27EE}', BracketType::Close),
    ('\u{2983}', '\u{2984}', BracketType::Open),
    ('\u{2984}', '\u{2983}', BracketType::Close),
    ('\u{2985}', '\u{2986}', BracketType::Open),
    ('\u{2986}', '\u{2985}', BracketType::Close),
    ('\u{2987}', '\u{2988}', BracketType::Open),
    ('\u{2988}', '\u{2987}', BracketType::Close),
    ('\u{2989}', '\u{298A}', BracketType::Open),
    ('\u{298A}', '\u{2989}', BracketType::Close),
    ('\u{298B}', '\u{298C}', BracketType::Open),
    ('\u{298C}', '\u{298B}', BracketType::Close),
    ('\u{298D}', '\u{2990}', BracketType::Open),
    ('\u{298E}', '\u{298F}', BracketType::Close),
    ('\u{298F}', '\u{298E}', BracketType::Open),
    ('\u{2990}', '\u{298D}', BracketType::Close),
    ('\u{2991}', '\u{2992}', BracketType::Open),
    ('\u{2992}', '\u{2991}', BracketType::Close),
    ('\u{2993}', '\u{2994}', BracketType::Open),
    ('\u{2994}', '\u{2993}', BracketType::Close),
    ('\u{2995}', '\u{2996}', BracketType::Open),
    ('\u{2996}', '\u{2995}', BracketType::Close),
    ('\u{2997}', '\u{2998}', BracketType::Open),
    ('\u{2998}', '\u{2997}', BracketType::Close),
    ('\u{29D8}', '\u{29D9}', BracketType::Open),
    ('\u{29D9}', '\u{29D8}', BracketType::Close),
    ('\u{29DA}', '\u{29DB}', BracketType::Open),
    ('\u{29DB}', '\u{29DA}', BracketType::Close),
    ('\u{29FC}', '\u{29FD}', BracketType::Open),
    ('\u{29FD}', '\u{29FC}', BracketType::Close),
    ('\u{2E22}', '\u{2E23}', BracketType::Open),
    ('\u{2E23}', '\u{2E22}', BracketType::Close),
    ('\u{2E24}', '\u{2E25}', BracketType::Open),
    ('\u{2E25}', '\u{2E24}', BracketType::Close),
    ('\u{2E26}', '\u{2E27}', BracketType::Open),
    ('\u{2E27}', '\u{2E26}', BracketType::Close),
    ('\u{2E28}', '\u{2E29}', BracketType::Open),
    ('\u{2E29}', '\u{2E28}', BracketType::Close),
    ('\u{2E55}', '\u{2E56}', BracketType::Open),
    ('\u{2E56}', '\u{2E55}', BracketType::Close),
    ('\u{2E57}', '\u{2E58}', BracketType::Open),
    ('\u{2E58}', '\u{2E57}', BracketType::Close),
    ('\u{2E59}', '\u{2E5A}', BracketType::Open),
    ('\u{2E5A}', '\u{2E59}', BracketType::Close),
    ('\u{2E5B}', '\u{2E5C}', BracketType::Open),
    ('\u{2E5C}', '\u{2E5B}', BracketType::Close),
    ('\u{3008}', '\u{3009}', BracketType::Open),
    ('\u{3009}', '\u{3008}', BracketType::Close),
    ('\u{300A}', '\u{300B}', BracketType::Open),
    ('\u{300B}', '\u{300A}', BracketType::Close),
    ('\u{300C}', '\u{300D}', BracketType::Open),
    ('\u{300D}', '\u{300C}', BracketType::Close),
    ('\u{300E}', '\u{300F}', BracketType::Open),
    ('\u{300F}', '\u{300E}', BracketType::Close),
    ('\u{3010}', '\u{3011}', BracketType::Open),
    ('\u{3011}', '\u{3010}', BracketType::Close),
    ('\u{3014}', '\u{3015}', BracketType::Open),
    ('\u{3015}', '\u{3014}', BracketType::Close),
    ('\u{3016}', '\u{3017}', BracketType::Open),
    ('\u{3017}', '\u{3016}', BracketType::Close),
    ('\u{3018}', '\u{3019}', BracketType::Open),
    ('\u{3019}', '\u{3018}', BracketType::Close),
    ('\u{301A}', '\u{301B}', BracketType::Open),
    ('\u{301B}', '\u{301A}', BracketType::Close),
    ('\u{FE59}', '\u{FE5A}', BracketType::Open),
    ('\u{FE5A}', '\u{FE59}', BracketType::Close),
    ('\u{FE5B}', '\u{FE5C}', BracketType::Open),
    ('\u{FE5C}', '\u{FE5B}', BracketType::Close),
    ('\u{FE5D}', '\u{FE5E}', BracketType::Open),
    ('\u{FE5E}', '\u{FE5D}', BracketType::Close),
    ('\u{FF08}', '\u{FF09}', BracketType::Open),
    ('\u{FF09}', '\u{FF08}', BracketType::Close),
    ('\u{FF3B}', '\u{FF3D}', BracketType::Open),
    ('\u{FF3D}', '\u{FF3B}', BracketType::Close),
    ('\u{FF5B}', '\u{FF5D}', BracketType::Open),
    ('\u{FF5D}', '\u{FF5B}', BracketType::Close),
    ('\u{FF5F}', '\u{FF60}', BracketType::Open),
    ('\u{FF60}', '\u{FF5F}', BracketType::Close),
    ('\u{FF62}', '\u{FF63}', BracketType::Open),
    ('\u{FF63}', '\u{FF62}', BracketType::Close),
];
