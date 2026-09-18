//! Compile-time diagnostics.

use crate::{NodeId, RowId, ast::Span};

/// Severity.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[allow(missing_docs)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// Lint codes, grouped by stage.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[allow(missing_docs)]
pub enum LintCode {
    // parse
    IncludeNotFound,
    IncludeCycle,
    UnknownDirective,
    PasteUnknownName,
    UnknownCallToken,
    SequenceNotFirst,
    IndentationMismatch,
    NonStandardToken,
    ColumnZeroContinuation,
    // expansion
    IllegalCall,
    UnboundOther,
    VariableNoCandidate,
    StepWithoutAnchor,
    WideWildcard,
    DuplicatePath,
    ShadowedByExact,
    ConditionTie,
    TooManyNodes,
    // constraints
    UnsatisfiableConstraint,
    ContradictsOwnHistory,
    LowRecognition,
    EmptyDescription,
    UnrecognizedFragment,
    SoftConstraint,
    AssumedContext,
    SiblingSubset,
    SiblingOverlap,
    DnfTruncated,
    // coverage
    MissingOpeningCoverage,
    MissingResponseCoverage,
}

/// One diagnostic.
#[derive(Clone, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Lint {
    /// Severity.
    pub severity: Severity,
    /// Code.
    pub code: LintCode,
    /// The row, if any.
    pub row: Option<RowId>,
    /// The node, if any.
    pub node: Option<NodeId>,
    /// Source location, if any.
    pub span: Option<Span>,
    /// Message.
    pub message: String,
}

impl core::fmt::Display for Lint {
    /// `file:line: severity[Code]: message`.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        todo!("phase 3")
    }
}

/// Summary counts for the compile-time `INFO` line.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct LintSummary {
    /// Errors.
    pub errors: usize,
    /// Warnings.
    pub warnings: usize,
    /// Infos.
    pub infos: usize,
}

impl LintSummary {
    /// Counts by severity.
    pub fn of(lints: &[Lint]) -> LintSummary {
        let mut s = LintSummary::default();
        for l in lints {
            match l.severity {
                Severity::Error => s.errors += 1,
                Severity::Warning => s.warnings += 1,
                Severity::Info => s.infos += 1,
            }
        }
        s
    }
}
