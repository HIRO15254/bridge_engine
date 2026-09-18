//! `#COPY` / `#CUT` / `#PASTE` (textual, in the reference implementation's order).

use crate::{Lint, ast::RawLine};

/// Expands clipboard directives inside one table paragraph: extract `#CUT` blocks, then `#COPY`
/// blocks (keeping their bodies), then expand every `#PASTE name tgt=rep …` by prepending the
/// paste line's indentation and applying the replacements in order.
pub fn expand(paragraph: Vec<RawLine>, clipboard: &mut Clipboard) -> (Vec<RawLine>, Vec<Lint>) {
    todo!("phase 3")
}

/// Named blocks available to `#PASTE` (global and order-dependent across the file).
#[derive(Clone, Debug, Default)]
pub struct Clipboard {
    /// `name → lines`.
    pub blocks: Vec<(String, Vec<RawLine>)>,
}
