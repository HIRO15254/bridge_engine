//! Clause grammar.
//!
//! ```ebnf
//! description = { line } ;
//! line        = enumitem | clauses ;
//! enumitem    = ( LETTER ")" | DIGIT ")" | DIGIT "." ) clauses ;      (* items form an OR group *)
//! clauses     = orgroup { ( "," | ";" | "." ) orgroup } ;              (* "," = AND, loosest *)
//! orgroup     = andgroup { ( "or" | "/" ) andgroup } ;
//! andgroup    = fragment { ( "and" | "with" | "w/" | "+" | WS ) fragment } ;
//! fragment    = [ negation ] [ hedge ] atom ;
//! negation    = "not" | "no" | "without" | "w/o" | "denies" | "non" ;
//! hedge       = "usually" | "normally" | "may" | "might" | "rarely" | "typically" | "(?)" ;
//! ```

use winnow::prelude::*;

/// A recognised or unrecognised piece of a description.
#[derive(Clone, PartialEq, Debug)]
pub struct Fragment {
    /// Byte span in the normalised text.
    pub span: (u16, u16),
    /// Negated.
    pub negated: bool,
    /// Hedged.
    pub hedged: bool,
    /// The content.
    pub kind: FragmentKind,
}

/// Fragment content (context-free where possible; see `tokens.rs` for the vocabulary).
#[derive(Clone, PartialEq, Debug)]
pub enum FragmentKind {
    /// An atom recognised by the token vocabulary.
    Token(super::tokens::Token),
    /// Text nobody recognised.
    Unrecognized(String),
}

/// The boolean structure of a description over fragment indices.
#[derive(Clone, PartialEq, Debug)]
pub enum Clause {
    /// A single fragment.
    Leaf(usize),
    /// Conjunction.
    And(Vec<Clause>),
    /// Disjunction.
    Or(Vec<Clause>),
}

/// Parses a normalised description into fragments and their boolean structure.
pub fn parse(text: &str) -> (Vec<Fragment>, Clause) {
    todo!("phase 3")
}

/// One fragment (winnow parser).
pub fn fragment(input: &mut &str) -> ModalResult<Fragment> {
    todo!("phase 3")
}
