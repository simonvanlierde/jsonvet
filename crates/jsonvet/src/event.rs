use std::borrow::Cow;

/// One step through a document's values, from [`parse`](crate::parse), in document order.
///
/// An object is `ObjectStart`, then a `Key` and its value for each member, then `End`; an array
/// is `ArrayStart`, its elements, then `End`. A JSON Lines file is one document after another.
///
/// Public for jsonvet-cli only and outside the semver contract: it can change in any release.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Event<'a> {
    /// The byte offset where the token starts: the bracket, the key or the value.
    pub offset: usize,
    /// What the token is.
    pub kind: EventKind<'a>,
}

/// The token an [`Event`] stands for. Strings come decoded, with U+FFFD for a lone surrogate
/// escape. Keys come decoded as WTF-8, which keeps each lone surrogate: see [`key_text`].
///
/// [`key_text`]: crate::key_text
///
/// Public for jsonvet-cli only and outside the semver contract: it can change in any release.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EventKind<'a> {
    /// `{`.
    ObjectStart,
    /// `[`.
    ArrayStart,
    /// The `}` or `]` that closes the innermost open container.
    End,
    /// An object member's key, as WTF-8: UTF-8, except that a lone surrogate escape such as
    /// `\ud800` is kept as its own three bytes (`ED A0 80`).
    Key(Cow<'a, [u8]>),
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A number as written, sign included: `-1.5e3`, and in JSON5 also `+0x1F`, `.5`, `5.`,
    /// `Infinity` or `NaN`.
    Number(&'a str),
    /// A string.
    String(Cow<'a, str>),
}
