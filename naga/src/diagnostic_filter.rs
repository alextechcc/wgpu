//! [`DiagnosticFilter`]s and supporting functionality.

#[cfg(feature = "wgsl-in")]
use crate::Span;
use crate::{Arena, Handle};
#[cfg(feature = "arbitrary")]
use arbitrary::Arbitrary;
#[cfg(feature = "wgsl-in")]
use indexmap::IndexMap;
#[cfg(feature = "deserialize")]
use serde::Deserialize;
#[cfg(feature = "serialize")]
use serde::Serialize;

/// A severity set on a [`DiagnosticFilter`].
///
/// <https://www.w3.org/TR/WGSL/#diagnostic-severity>
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[cfg_attr(feature = "serialize", derive(Serialize))]
#[cfg_attr(feature = "deserialize", derive(Deserialize))]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
pub enum Severity {
    Off,
    Info,
    Warning,
    Error,
}

impl Severity {
    const ERROR: &'static str = "error";
    const WARNING: &'static str = "warning";
    const INFO: &'static str = "info";
    const OFF: &'static str = "off";

    /// Convert from a sentinel word in WGSL into its associated [`Severity`], if possible.
    pub fn from_ident(s: &str) -> Option<Self> {
        Some(match s {
            Self::ERROR => Self::Error,
            Self::WARNING => Self::Warning,
            Self::INFO => Self::Info,
            Self::OFF => Self::Off,
            _ => return None,
        })
    }

    /// Checks whether this severity is [`Self::Error`].
    ///
    /// Naga does not yet support diagnostic items at lesser severities than
    /// [`Severity::Error`]. When this is implemented, this method should be deleted, and the
    /// severity should be used directly for reporting diagnostics.
    pub(crate) fn report_diag<E>(
        self,
        err: E,
        log_handler: impl FnOnce(E, log::Level),
    ) -> Result<(), E> {
        let log_level = match self {
            Severity::Off => return Ok(()),

            // NOTE: These severities are not yet reported.
            Severity::Info => log::Level::Info,
            Severity::Warning => log::Level::Warn,

            Severity::Error => return Err(err),
        };
        log_handler(err, log_level);
        Ok(())
    }
}

/// A filterable triggering rule in a [`DiagnosticFilter`].
///
/// <https://www.w3.org/TR/WGSL/#filterable-triggering-rules>
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[cfg_attr(feature = "serialize", derive(Serialize))]
#[cfg_attr(feature = "deserialize", derive(Deserialize))]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
pub enum FilterableTriggeringRule {
    DerivativeUniformity,
}

impl FilterableTriggeringRule {
    const DERIVATIVE_UNIFORMITY: &'static str = "derivative_uniformity";

    /// Convert from a sentinel word in WGSL into its associated [`FilterableTriggeringRule`], if possible.
    pub fn from_ident(s: &str) -> Option<Self> {
        Some(match s {
            Self::DERIVATIVE_UNIFORMITY => Self::DerivativeUniformity,
            _ => return None,
        })
    }

    /// Maps this [`FilterableTriggeringRule`] into the sentinel word associated with it in WGSL.
    pub const fn to_ident(self) -> &'static str {
        match self {
            Self::DerivativeUniformity => Self::DERIVATIVE_UNIFORMITY,
        }
    }

    /// The default severity associated with this triggering rule.
    ///
    /// See <https://www.w3.org/TR/WGSL/#filterable-triggering-rules> for a table of default
    /// severities.
    pub(crate) const fn default_severity(self) -> Severity {
        match self {
            FilterableTriggeringRule::DerivativeUniformity => Severity::Error,
        }
    }
}

/// A filter that modifies how diagnostics are emitted for shaders.
///
/// <https://www.w3.org/TR/WGSL/#diagnostic-filter>
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serialize", derive(Serialize))]
#[cfg_attr(feature = "deserialize", derive(Deserialize))]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
pub struct DiagnosticFilter {
    pub new_severity: Severity,
    pub triggering_rule: FilterableTriggeringRule,
}

/// A map of diagnostic filters to their severity and first occurrence's span.
///
/// Intended for front ends' first step into storing parsed [`DiagnosticFilter`]s.
#[derive(Clone, Debug, Default)]
#[cfg(feature = "wgsl-in")]
pub(crate) struct DiagnosticFilterMap(IndexMap<FilterableTriggeringRule, (Severity, Span)>);

#[cfg(feature = "wgsl-in")]
impl DiagnosticFilterMap {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Add the given `diagnostic_filter` parsed at the given `span` to this map.
    pub(crate) fn add(
        &mut self,
        diagnostic_filter: DiagnosticFilter,
        span: Span,
    ) -> Result<(), ConflictingDiagnosticRuleError> {
        use indexmap::map::Entry;

        let &mut Self(ref mut diagnostic_filters) = self;
        let DiagnosticFilter {
            new_severity,
            triggering_rule,
        } = diagnostic_filter;

        match diagnostic_filters.entry(triggering_rule) {
            Entry::Vacant(entry) => {
                entry.insert((new_severity, span));
            }
            Entry::Occupied(entry) => {
                let &(first_severity, first_span) = entry.get();
                if first_severity != new_severity {
                    return Err(ConflictingDiagnosticRuleError {
                        triggering_rule,
                        triggering_rule_spans: [first_span, span],
                    });
                }
            }
        }
        Ok(())
    }
}

#[cfg(feature = "wgsl-in")]
impl IntoIterator for DiagnosticFilterMap {
    type Item = (FilterableTriggeringRule, (Severity, Span));

    type IntoIter = indexmap::map::IntoIter<FilterableTriggeringRule, (Severity, Span)>;

    fn into_iter(self) -> Self::IntoIter {
        let Self(this) = self;
        this.into_iter()
    }
}

/// An error returned by [`DiagnosticFilterMap::add`] when it encounters conflicting rules.
#[cfg(feature = "wgsl-in")]
#[derive(Clone, Debug)]
pub(crate) struct ConflictingDiagnosticRuleError {
    pub triggering_rule: FilterableTriggeringRule,
    pub triggering_rule_spans: [Span; 2],
}

/// Represents a single parent-linking node in a tree of [`DiagnosticFilter`]s backed by a
/// [`crate::Arena`].
///
/// A single element of a _tree_ of diagnostic filter rules stored in
/// [`crate::Module::diagnostic_filters`]. When nodes are built by a front-end, module-applicable
/// filter rules are chained together in runs based on parse site.  For instance, given the
/// following:
///
/// - Module-applicable rules `a` and `b`.
/// - Rules `c` and `d`, applicable to an entry point called `c_and_d_func`.
/// - Rule `e`, applicable to an entry point called `e_func`.
///
/// The tree would be represented as follows:
///
/// ```text
/// a <- b
///      ^
///      |- c <- d
///      |
///      \- e
/// ```
///
/// ...where:
///
/// - `d` is the first leaf consulted by validation in `c_and_d_func`.
/// - `e` is the first leaf consulted by validation in `e_func`.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serialize", derive(Serialize))]
#[cfg_attr(feature = "deserialize", derive(Deserialize))]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
pub struct DiagnosticFilterNode {
    pub inner: DiagnosticFilter,
    pub parent: Option<Handle<DiagnosticFilterNode>>,
}

impl DiagnosticFilterNode {
    /// Finds the most specific filter rule applicable to `triggering_rule` from the chain of
    /// diagnostic filter rules in `arena`, starting with `node`, and returns its severity. If none
    /// is found, return the value of [`FilterableTriggeringRule::default_severity`].
    ///
    /// When `triggering_rule` is not applicable to this node, its parent is consulted recursively.
    pub(crate) fn search(
        node: Option<Handle<Self>>,
        arena: &Arena<Self>,
        triggering_rule: FilterableTriggeringRule,
    ) -> Severity {
        let mut next = node;
        while let Some(handle) = next {
            let node = &arena[handle];
            let &Self { ref inner, parent } = node;
            let &DiagnosticFilter {
                triggering_rule: rule,
                new_severity,
            } = inner;

            if rule == triggering_rule {
                return new_severity;
            }

            next = parent;
        }
        triggering_rule.default_severity()
    }
}
