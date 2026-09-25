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

impl core::fmt::Display for Severity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
        })
    }
}

impl Lint {
    /// A new lint with no row, node or span attached yet.
    pub fn new(severity: Severity, code: LintCode, message: impl Into<String>) -> Lint {
        Lint {
            severity,
            code,
            row: None,
            node: None,
            span: None,
            message: message.into(),
        }
    }

    /// An [`Severity::Error`] lint.
    pub fn error(code: LintCode, message: impl Into<String>) -> Lint {
        Lint::new(Severity::Error, code, message)
    }

    /// A [`Severity::Warning`] lint.
    pub fn warning(code: LintCode, message: impl Into<String>) -> Lint {
        Lint::new(Severity::Warning, code, message)
    }

    /// An [`Severity::Info`] lint.
    pub fn info(code: LintCode, message: impl Into<String>) -> Lint {
        Lint::new(Severity::Info, code, message)
    }

    /// Attaches a source location.
    #[must_use]
    pub fn with_span(mut self, span: Span) -> Lint {
        self.span = Some(span);
        self
    }

    /// Attaches the row this lint concerns.
    #[must_use]
    pub fn with_row(mut self, row: RowId) -> Lint {
        self.row = Some(row);
        self
    }

    /// Attaches the node this lint concerns.
    #[must_use]
    pub fn with_node(mut self, node: NodeId) -> Lint {
        self.node = Some(node);
        self
    }
}

impl core::fmt::Display for Lint {
    /// `file:line: severity[Code]: message`.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match &self.span {
            Some(span) => write!(
                f,
                "{}:{}: {}[{:?}]: {}",
                span.file.0, span.line, self.severity, self.code, self.message
            ),
            None => write!(
                f,
                "<no location>: {}[{:?}]: {}",
                self.severity, self.code, self.message
            ),
        }
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
