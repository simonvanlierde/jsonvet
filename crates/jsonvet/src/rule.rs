use std::fmt;
use std::str::FromStr;

use crate::Dialect;

/// What jsonvet checks. Every problem belongs to one rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Rule {
    /// The first syntax error. Checking stops there, and its severity is always error.
    Syntax,
    /// A key repeated in the same object.
    DuplicateKey,
    /// A number outside a double's range, or an integer past ±(2⁵³ − 1).
    InvalidNumber,
    /// A lone surrogate or a noncharacter in a string.
    InvalidCodePoint,
    /// A byte order mark at the start of the file.
    ByteOrderMark,
    /// A JSON Lines file that does not end with a newline.
    MissingFinalNewline,
}

impl Rule {
    /// Every rule, in the order the docs list them.
    pub const ALL: &'static [Rule] = &[
        Rule::Syntax,
        Rule::DuplicateKey,
        Rule::InvalidNumber,
        Rule::InvalidCodePoint,
        Rule::ByteOrderMark,
        Rule::MissingFinalNewline,
    ];

    /// The rule's id, such as `duplicate-key`.
    pub fn id(self) -> &'static str {
        match self {
            Rule::Syntax => "syntax",
            Rule::DuplicateKey => "duplicate-key",
            Rule::InvalidNumber => "invalid-number",
            Rule::InvalidCodePoint => "invalid-code-point",
            Rule::ByteOrderMark => "byte-order-mark",
            Rule::MissingFinalNewline => "missing-final-newline",
        }
    }

    /// A one-line description of what the rule reports, such as `a key repeated in one object`.
    pub fn description(self) -> &'static str {
        match self {
            Rule::Syntax => "the first syntax error; checking stops there",
            Rule::DuplicateKey => "a key repeated in one object",
            Rule::InvalidNumber => "a number a double cannot hold, or an integer past 2^53 - 1",
            Rule::InvalidCodePoint => "a lone surrogate or a noncharacter in a string",
            Rule::ByteOrderMark => "a byte order mark at the start of the file",
            Rule::MissingFinalNewline => "a JSON Lines file that does not end with a newline",
        }
    }

    /// The rule's severity when nothing overrides it.
    pub fn default_severity(self, dialect: Dialect) -> Severity {
        match (self, dialect) {
            (Rule::Syntax | Rule::DuplicateKey, _)
            | (Rule::InvalidNumber | Rule::InvalidCodePoint, Dialect::IJson)
            | (Rule::ByteOrderMark, Dialect::JsonLines) => Severity::Error,
            (Rule::ByteOrderMark, Dialect::Json | Dialect::IJson)
            | (Rule::MissingFinalNewline, Dialect::JsonLines) => Severity::Warning,
            _ => Severity::Off,
        }
    }
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// The error for a rule id jsonvet does not know.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownRule(String);

impl UnknownRule {
    /// The id that was given.
    pub fn id(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for UnknownRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ids: Vec<&str> = Rule::ALL.iter().map(|r| r.id()).collect();
        write!(
            f,
            "unknown rule `{}`{}; expected one of {}",
            self.0,
            crate::did_you_mean(&self.0, ids.iter().copied()),
            ids.join(", ")
        )
    }
}

impl std::error::Error for UnknownRule {}

impl FromStr for Rule {
    type Err = UnknownRule;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Rule::ALL
            .iter()
            .copied()
            .find(|r| r.id() == s)
            .ok_or_else(|| UnknownRule(s.to_owned()))
    }
}

/// How much a problem matters. An ignored (`Off`) rule is not checked at all. Severities are
/// ordered from `Off` to `Error`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Severity {
    /// Not checked.
    Off,
    /// Reported, but does not fail the run unless a warning limit is set.
    Warning,
    /// Reported, and fails the run.
    Error,
}

impl Severity {
    /// The name used in output and config files: `off`, `warning` or `error`.
    pub fn name(self) -> &'static str {
        match self {
            Severity::Off => "off",
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }

    /// True unless the rule is `Off`, so its problems get checked and reported.
    pub(crate) fn is_on(self) -> bool {
        self != Severity::Off
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// What to check: a dialect, and any rules whose severity differs from the dialect's default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    dialect: Dialect,
    overrides: [Option<Severity>; Rule::ALL.len()],
    pub(crate) max_depth: usize,
}

impl Config {
    /// How deep objects and arrays may nest by default: about where check-json5 1.4 stopped
    /// (9,998 levels), so files it passed still pass, and the same as Go's `encoding/json`.
    pub const DEFAULT_MAX_DEPTH: usize = 10_000;

    /// The dialect's defaults.
    pub fn new(dialect: Dialect) -> Self {
        Config {
            dialect,
            overrides: [None; Rule::ALL.len()],
            max_depth: Self::DEFAULT_MAX_DEPTH,
        }
    }

    /// Set how deep objects and arrays may nest; one level deeper is a syntax error.
    #[must_use]
    pub fn max_depth(mut self, depth: usize) -> Self {
        self.max_depth = depth;
        self
    }

    /// Set one rule's severity. `Rule::Syntax` stays an error whatever is set here.
    #[must_use]
    pub fn rule(mut self, rule: Rule, severity: Severity) -> Self {
        self.overrides[rule as usize] = Some(severity);
        self
    }

    /// The dialect being checked.
    pub fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// A rule's severity under this config.
    pub fn severity(&self, rule: Rule) -> Severity {
        if rule == Rule::Syntax {
            return Severity::Error;
        }
        self.overrides[rule as usize].unwrap_or_else(|| rule.default_severity(self.dialect))
    }
}

#[cfg(test)]
mod tests {
    use super::Rule;

    #[test]
    fn all_lists_every_rule_at_its_index() {
        // `Config` indexes its overrides by `rule as usize`, so ALL must hold every rule, in
        // declaration order.
        for (i, &rule) in Rule::ALL.iter().enumerate() {
            assert_eq!(rule as usize, i, "{rule}");
        }
        let last = Rule::MissingFinalNewline;
        // No wildcard: a new rule stops this building until `last` and ALL are updated.
        match last {
            Rule::Syntax
            | Rule::DuplicateKey
            | Rule::InvalidNumber
            | Rule::InvalidCodePoint
            | Rule::ByteOrderMark
            | Rule::MissingFinalNewline => {}
        }
        assert_eq!(Rule::ALL.len(), last as usize + 1);
    }
}
