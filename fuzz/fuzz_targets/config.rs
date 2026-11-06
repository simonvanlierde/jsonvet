//! A dialect, a rule config and a file name chosen by the input, applied to the rest of it: the
//! names round-trip through `FromStr`, `for_path` agrees with the tables, a rule set to `Off`
//! reports nothing, a severity override shows in every problem, `parse_with` streams what `parse`
//! returns, and the report keeps `common::report_violation`'s promises under any config.
#![no_main]

use std::path::Path;

use jsonvet::{Config, Dialect, Rule, Severity, parse, parse_lines, parse_with};
use libfuzzer_sys::arbitrary::{Result, Unstructured};
use libfuzzer_sys::fuzz_target;

#[path = "../../crates/jsonvet/tests/common/mod.rs"]
mod common;

fn severity(u: &mut Unstructured) -> Result<Option<Severity>> {
    Ok(match u.int_in_range(0..=3)? {
        0 => Some(Severity::Off),
        1 => Some(Severity::Warning),
        2 => Some(Severity::Error),
        _ => None,
    })
}

fn run(data: &[u8]) -> Result<()> {
    let mut u = Unstructured::new(data);
    let dialect = *u.choose(Dialect::ALL)?;
    let mut config = Config::new(dialect);
    if u.arbitrary()? {
        config = config.max_depth(u.int_in_range(0..=12)?);
    }
    for &rule in Rule::ALL {
        if let Some(set) = severity(&mut u)? {
            config = config.rule(rule, set);
        }
    }
    // A name made from the tables (so the extension and alias lookups hit) with a free tail, or
    // a free name.
    let name = if u.arbitrary()? {
        let known: Vec<&str> = Dialect::EXTENSIONS
            .iter()
            .chain(Dialect::FILE_NAMES)
            .chain(Dialect::ALIASES)
            .map(|&(name, _)| name)
            .chain(Dialect::ALL.iter().map(|d| d.name()))
            .chain(Rule::ALL.iter().map(|r| r.id()))
            .collect();
        format!("{}{}", u.choose(&known)?, u.arbitrary::<&str>()?)
    } else {
        u.arbitrary::<&str>()?.to_owned()
    };
    let text = String::from_utf8_lossy(u.take_rest());

    // Names.
    for &d in Dialect::ALL {
        assert_eq!(d.name().parse::<Dialect>(), Ok(d));
        assert_eq!(d.to_string(), d.name());
    }
    for &(alias, d) in Dialect::ALIASES {
        assert_eq!(alias.parse::<Dialect>(), Ok(d));
    }
    for &r in Rule::ALL {
        assert_eq!(r.id().parse::<Rule>(), Ok(r));
        assert!(!r.description().is_empty());
        assert_eq!(r.to_string(), r.id());
    }
    if let Err(e) = name.parse::<Dialect>() {
        assert_eq!(e.name(), name);
        assert!(e.to_string().contains(&name));
    }
    if let Err(e) = name.parse::<Rule>() {
        assert_eq!(e.id(), name);
        assert!(e.to_string().contains(&name));
    }
    let path = Path::new(&name);
    if let Some(found) = Dialect::for_path(path) {
        let base = path.file_name().and_then(|n| n.to_str()).expect("a name");
        let by_name = Dialect::FILE_NAMES
            .iter()
            .any(|&(n, d)| n == base && d == found);
        let by_ext = base
            .rsplit_once('.')
            .is_some_and(|(_, ext)| Dialect::EXTENSIONS.contains(&(ext, found)));
        assert!(by_name || by_ext, "{name:?} -> {found}");
    }

    // The config.
    assert_eq!(config.severity(Rule::Syntax), Severity::Error);
    let report = jsonvet::check(&text, &config);
    for problem in report.problems() {
        let rule = problem.rule();
        assert_ne!(
            config.severity(rule),
            Severity::Off,
            "{rule} is off: {problem:?}"
        );
        let severity = problem.severity();
        assert_eq!(severity, config.severity(rule), "{problem:?}");
        assert_eq!(severity.to_string(), severity.name());
    }
    if let Err(broken) = common::report_violation(&text, &config) {
        panic!("{text:?}: {broken}");
    }

    // The streaming parse.
    let (parsed, events) = parse(&text, &config);
    let mut streamed = Vec::new();
    let streaming = parse_with(&text, &config, &mut |e| streamed.push(e));
    assert_eq!(streaming.problems(), parsed.problems(), "{text:?}");
    if let Some(events) = events {
        assert_eq!(streamed, events, "{text:?}");
    }
    let rebuilt = jsonvet::report(&text, parsed.problems().to_vec(), &config);
    assert_eq!(rebuilt.problems(), parsed.problems());
    if dialect == Dialect::JsonLines {
        let whole = parse_lines(&text, 0..text.len(), &config, None);
        let syntax = parsed.problems().iter().any(|p| p.rule() == Rule::Syntax);
        assert_eq!(whole.syntax_error(), syntax, "{text:?}");
    }
    Ok(())
}

fuzz_target!(|data: &[u8]| {
    // A short input that cannot pick a config is not a failure.
    let _ = run(data);
});
