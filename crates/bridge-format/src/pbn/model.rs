//! The PBN data model: tags, sections and tokens as written, before interpretation.

use bridge_core::{Call, Card};

/// A parsed PBN file.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct PbnFile {
    /// Games in file order.
    pub games: Vec<Game>,
    /// `%` escape lines (`% PBN 2.1`, `% EXPORT`, others verbatim).
    pub directives: Vec<Directive>,
}

/// A `%` escape line.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Directive {
    /// `% PBN <major>.<minor>`.
    Version(u8, u8),
    /// `% EXPORT`.
    Export,
    /// Any other directive, kept verbatim.
    Other(String),
}

/// One game (one board record).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Game {
    /// Tag pairs in file order.
    pub tags: Vec<TagPair>,
    /// Auction, play and table sections.
    pub sections: Vec<Section>,
    /// `{…}` and `;` commentary with its anchor.
    pub commentary: Vec<Comment>,
}

impl Game {
    /// The value of the first tag named `name` (case-sensitive, as PBN requires).
    pub fn get(&self, name: &str) -> Option<&TagValue> {
        self.tags.iter().find(|t| t.name == name).map(|t| &t.value)
    }
}

/// A `[Name "value"]` pair.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TagPair {
    /// Tag name.
    pub name: String,
    /// Tag value.
    pub value: TagValue,
    /// 1-based line.
    pub line: u32,
}

/// A tag value.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TagValue {
    /// A string.
    Str(String),
    /// `#`: inherited from the previous game.
    Inherited,
    /// `##x`: inherited from the previous game with default `x`.
    Default(String),
}

/// The token lines following an `Auction`, `Play` or `*Table` tag.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Section {
    /// The tag name (`Auction`, `Play`, `OptimumResultTable`, …).
    pub tag: String,
    /// The tag argument (the first seat for `Auction`/`Play`, the column spec for tables).
    pub arg: String,
    /// Tokens in order.
    pub tokens: Vec<Token>,
    /// `[Note "n:text"]` tags attached to this section.
    pub notes: Vec<(u8, String)>,
}

/// A section token.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Token {
    /// A call (auction sections).
    Call(Call),
    /// A card (play sections).
    Card(Card),
    /// `-`: unknown call or card.
    Unknown,
    /// `*`: the section is complete.
    Terminator,
    /// `+`: the section continues (import only).
    Continuation,
    /// `=n=`: reference to note `n`.
    NoteRef(u8),
    /// `$n`: numeric annotation glyph.
    Nag(u8),
    /// `!`, `?`, `!!`, `??`, `!?`, `?!`.
    Suffix(String),
    /// Anything else (table cells, unparsed text).
    Raw(String),
}

/// Commentary with its anchor.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Comment {
    /// The text without braces.
    pub text: String,
    /// Index of the tag pair the comment follows, or `None` for a leading comment.
    pub after_tag: Option<usize>,
    /// 1-based line.
    pub line: u32,
}

/// What a section's tokens are.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SectionKind {
    Auction,
    Play,
    /// `*Table`: rows kept as [`Token::Raw`].
    Table,
}

/// Whether a tag of this name opens a section.
pub(crate) fn is_section_tag(name: &str) -> bool {
    name == "Auction" || name == "Play" || (name.len() > 5 && name.ends_with("Table"))
}

pub(crate) fn section_kind(tag: &str) -> SectionKind {
    match tag {
        "Auction" => SectionKind::Auction,
        "Play" => SectionKind::Play,
        _ => SectionKind::Table,
    }
}
