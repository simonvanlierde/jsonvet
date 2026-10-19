use std::fmt;
use std::path::Path;
use std::str::FromStr;

/// A member of the JSON family. In the order JSON, JSONC, JWCC, JSON5, each one adds syntax to the
/// one before it. The one thing that does not carry over is where a `//` comment ends: JWCC ends
/// it at LF only, JSONC also at a lone CR, and JSON5 also at U+2028 and U+2029. I-JSON is JSON
/// with stricter content rules, and JSON Lines is one JSON value per line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Dialect {
    /// JSON as RFC 8259 defines it.
    Json,
    /// I-JSON (RFC 7493): JSON whose lone surrogates, noncharacters and out-of-range numbers
    /// are errors. Duplicate keys are an error in every dialect.
    IJson,
    /// JSON with `//` and `/* */` comments.
    Jsonc,
    /// JSON with comments and trailing commas, also called HuJSON.
    Jwcc,
    /// JSON5 1.0.0.
    Json5,
    /// JSON Lines: one JSON value per line, also called NDJSON.
    JsonLines,
}

impl Dialect {
    /// Every dialect, in the order the docs list them.
    pub const ALL: &'static [Dialect] = &[
        Dialect::Json,
        Dialect::IJson,
        Dialect::Jsonc,
        Dialect::Jwcc,
        Dialect::Json5,
        Dialect::JsonLines,
    ];

    /// The name used on the command line and in config files, such as `jsonc`.
    pub fn name(self) -> &'static str {
        match self {
            Dialect::Json => "json",
            Dialect::IJson => "i-json",
            Dialect::Jsonc => "jsonc",
            Dialect::Jwcc => "jwcc",
            Dialect::Json5 => "json5",
            Dialect::JsonLines => "jsonl",
        }
    }

    /// Other names [`FromStr`] accepts, each with the dialect it stands for.
    pub const ALIASES: &'static [(&'static str, Dialect)] = &[
        ("ijson", Dialect::IJson),
        ("hujson", Dialect::Jwcc),
        ("ndjson", Dialect::JsonLines),
        ("json-lines", Dialect::JsonLines),
    ];

    /// The file extensions jsonvet checks by default, without the dot, each with its dialect.
    /// Matching is case-sensitive.
    pub const EXTENSIONS: &'static [(&'static str, Dialect)] = &[
        ("json", Dialect::Json5),
        ("json5", Dialect::Json5),
        ("geojson", Dialect::Json5),
        ("jsonld", Dialect::Json5),
        ("webmanifest", Dialect::Json5),
        ("jsonc", Dialect::Jsonc),
        ("code-workspace", Dialect::Jsonc),
        ("code-snippets", Dialect::Jsonc),
        ("hujson", Dialect::Jwcc),
        ("jwcc", Dialect::Jwcc),
        ("jsonl", Dialect::JsonLines),
        ("ndjson", Dialect::JsonLines),
    ];

    /// Whole file names jsonvet checks by default, each with its dialect. These match the
    /// complete name only: `.babelrc`, not `a.babelrc`.
    pub const FILE_NAMES: &'static [(&'static str, Dialect)] = &[(".babelrc", Dialect::Json5)];

    /// The default dialect for a file, from [`FILE_NAMES`](Self::FILE_NAMES) or else
    /// [`EXTENSIONS`](Self::EXTENSIONS), or `None` for a file jsonvet does not check by default.
    /// `.json` files default to JSON5, because most `.json` files that fail as JSON are config
    /// files with comments.
    pub fn for_path(path: &Path) -> Option<Dialect> {
        let name = path.file_name()?.to_str()?;
        let (_, ext) = name.rsplit_once('.').unwrap_or_default();
        let find = |table: &[(&str, Dialect)], key: &str| {
            table.iter().find(|(k, _)| *k == key).map(|&(_, d)| d)
        };
        find(Self::FILE_NAMES, name).or_else(|| find(Self::EXTENSIONS, ext))
    }

    pub(crate) fn comments(self) -> bool {
        matches!(self, Dialect::Jsonc | Dialect::Jwcc | Dialect::Json5)
    }

    pub(crate) fn trailing_commas(self) -> bool {
        matches!(self, Dialect::Jwcc | Dialect::Json5)
    }

    /// Whether a `//` comment ends only at `\n`. JWCC's text says `\r` bytes are irrelevant.
    pub(crate) fn line_comment_ends_only_at_lf(self) -> bool {
        self == Dialect::Jwcc
    }

    pub(crate) fn json5(self) -> bool {
        self == Dialect::Json5
    }
}

impl fmt::Display for Dialect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The error for a dialect name jsonvet does not know.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownDialect(String);

impl UnknownDialect {
    /// The name that was given.
    pub fn name(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for UnknownDialect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = Dialect::ALL
            .iter()
            .map(|d| d.name())
            .chain(Dialect::ALIASES.iter().map(|&(alias, _)| alias))
            .collect();
        write!(
            f,
            "unknown dialect `{}`{}; expected one of {}",
            self.0,
            crate::did_you_mean(&self.0, names.iter().copied()),
            names.join(", ")
        )
    }
}

impl std::error::Error for UnknownDialect {}

impl FromStr for Dialect {
    type Err = UnknownDialect;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Dialect::ALL
            .iter()
            .map(|&d| (d.name(), d))
            .chain(Dialect::ALIASES.iter().copied())
            .find(|&(name, _)| name == s)
            .map(|(_, d)| d)
            .ok_or_else(|| UnknownDialect(s.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::Dialect;

    #[test]
    fn all_lists_every_dialect_at_its_index() {
        for (i, &dialect) in Dialect::ALL.iter().enumerate() {
            assert_eq!(dialect as usize, i, "{dialect}");
        }
        let last = Dialect::JsonLines;
        // No wildcard: a new dialect stops this building until `last` and ALL are updated.
        match last {
            Dialect::Json
            | Dialect::IJson
            | Dialect::Jsonc
            | Dialect::Jwcc
            | Dialect::Json5
            | Dialect::JsonLines => {}
        }
        assert_eq!(Dialect::ALL.len(), last as usize + 1);
    }
}
