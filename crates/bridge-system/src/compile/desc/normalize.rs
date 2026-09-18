//! Normalisation of description text.

/// A normalised description.
#[derive(Clone, PartialEq, Debug)]
pub struct Normalized {
    /// Text with suit sentinels (`♣♦♥♠`), `-` ranges and collapsed spaces; line breaks kept.
    pub text: String,
    /// A leading `!` marker was present.
    pub alert: bool,
    /// `{prio:N}`.
    pub priority: Option<i16>,
    /// `{w:X}` values in order of appearance.
    pub weights: Vec<f32>,
    /// Map from normalised byte offsets back to original offsets (for spans).
    pub offsets: Vec<u16>,
}

/// Normalises `text`.
pub fn normalize(text: &str) -> Normalized {
    todo!("phase 3")
}
