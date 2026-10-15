//! Diagnostics with the math string, its location in the math and a caret on the span.

use std::fmt;

use miette::{LabeledSpan, NamedSource, Severity, SourceSpan};

use crate::ast::Span;

/// Where a math string lives: `constraints.foo.equations[0].expression` and the like.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StringPath {
    /// Component group (`constraints`, `variables`, ...).
    pub group: String,
    /// Component name.
    pub component: String,
    /// Position in the component, as Calliope's parser reports it
    /// (`where`, `equations[0].expression`, `sub_expressions.foo[1].where`, ...).
    pub field: String,
}

impl fmt::Display for StringPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.group, self.component, self.field)
    }
}

/// One finding about the math.
#[derive(Debug, Clone, PartialEq)]
pub struct MathDiagnostic {
    /// Error or warning.
    pub severity: Severity,
    /// Stable short code (`math::dims::not-in-foreach`, ...).
    pub code: &'static str,
    /// What is wrong.
    pub message: String,
    /// The math string it is about.
    pub path: StringPath,
    /// The math string.
    pub source: String,
    /// The offending part of the string.
    pub span: Span,
    /// The span's label.
    pub label: String,
    /// Extra advice.
    pub help: Option<String>,
}

impl MathDiagnostic {
    /// An error.
    pub fn error(
        code: &'static str,
        path: StringPath,
        source: &str,
        span: Span,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity: Severity::Error,
            code,
            message: message.into(),
            path,
            source: source.to_owned(),
            span,
            label: String::new(),
            help: None,
        }
    }

    /// A warning.
    pub fn warning(
        code: &'static str,
        path: StringPath,
        source: &str,
        span: Span,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity: Severity::Warning,
            ..Self::error(code, path, source, span, message)
        }
    }

    /// With a label on the span.
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// With advice.
    #[must_use]
    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// True for errors.
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    /// One line: `path: message [`snippet`]`.
    pub fn short(&self) -> String {
        let snippet = self.source.get(self.span.clone()).unwrap_or("");
        format!("{}: {} [`{snippet}`]", self.path, self.message)
    }
}

impl fmt::Display for MathDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} | {}", self.path, self.message)
    }
}

impl std::error::Error for MathDiagnostic {}

impl miette::Diagnostic for MathDiagnostic {
    fn code<'a>(&'a self) -> Option<Box<dyn fmt::Display + 'a>> {
        Some(Box::new(self.code))
    }
    fn severity(&self) -> Option<Severity> {
        Some(self.severity)
    }
    fn help<'a>(&'a self) -> Option<Box<dyn fmt::Display + 'a>> {
        self.help
            .as_ref()
            .map(|h| Box::new(h) as Box<dyn fmt::Display>)
    }
    fn source_code(&self) -> Option<&dyn miette::SourceCode> {
        Some(self)
    }
    fn labels(&self) -> Option<Box<dyn Iterator<Item = LabeledSpan> + '_>> {
        let label = (!self.label.is_empty()).then(|| self.label.clone());
        let span = SourceSpan::from(self.span.start..self.span.end.max(self.span.start));
        Some(Box::new(std::iter::once(LabeledSpan::new_with_span(
            label, span,
        ))))
    }
}

impl miette::SourceCode for MathDiagnostic {
    fn read_span<'a>(
        &'a self,
        span: &SourceSpan,
        context_lines_before: usize,
        context_lines_after: usize,
    ) -> Result<Box<dyn miette::SpanContents<'a> + 'a>, miette::MietteError> {
        let contents = self
            .source
            .read_span(span, context_lines_before, context_lines_after)?;
        Ok(Box::new(miette::MietteSpanContents::new_named(
            self.path.to_string(),
            contents.data(),
            *contents.span(),
            contents.line(),
            contents.column(),
            contents.line_count(),
        )))
    }
}

/// A source for rendering with miette when the caller wants a [`NamedSource`].
pub fn named_source(d: &MathDiagnostic) -> NamedSource<String> {
    NamedSource::new(d.path.to_string(), d.source.clone())
}
