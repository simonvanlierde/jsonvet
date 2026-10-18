//! Any text, every dialect: `parse` does not panic, reports what `check` reports, returns no
//! events exactly when there is a syntax error, and starts every event on a char boundary inside
//! the text. For text that both serde_json and jsonvet accept as JSON, the events fold to the
//! value serde_json reads.
#![no_main]

use jsonvet::{Config, Dialect, EventKind, Rule, check, key_text, parse};
use libfuzzer_sys::fuzz_target;
use serde_json::{Map, Value};

/// The value one document's events describe. Numbers go through serde_json, so they compare
/// as serde_json would read them in place; a later duplicate key replaces the earlier one, as
/// in serde_json. A loop, not recursion, so deep nesting cannot overflow the stack.
fn fold(events: &[jsonvet::Event]) -> Value {
    // Each open container, with the key its next value goes under when it is an object.
    let mut stack: Vec<(Value, Option<String>)> = Vec::new();
    for event in events {
        let value = match &event.kind {
            EventKind::ObjectStart => {
                stack.push((Value::Object(Map::new()), None));
                continue;
            }
            EventKind::ArrayStart => {
                stack.push((Value::Array(Vec::new()), None));
                continue;
            }
            EventKind::Key(key) => {
                stack.last_mut().expect("key outside an object").1 = Some(key_text(key).into());
                continue;
            }
            EventKind::End => stack.pop().expect("end without a start").0,
            EventKind::Null => Value::Null,
            EventKind::Bool(b) => Value::Bool(*b),
            EventKind::Number(n) => serde_json::from_str(n).expect("number serde_json reads"),
            EventKind::String(s) => Value::String(s.to_string()),
            other => panic!("unexpected event {other:?}"),
        };
        match stack.last_mut() {
            None => return value,
            Some((Value::Array(items), _)) => items.push(value),
            Some((Value::Object(members), key)) => {
                members.insert(key.take().expect("value without a key"), value);
            }
            Some(_) => unreachable!(),
        }
    }
    panic!("events end before the document does: {events:?}")
}

fuzz_target!(|text: &str| {
    for &dialect in Dialect::ALL {
        let config = Config::new(dialect);
        let (report, events) = parse(text, &config);
        let checked = check(text, &config);
        assert_eq!(report.problems(), checked.problems(), "{dialect} {text:?}");
        let syntax_error = checked.problems().iter().any(|p| p.rule() == Rule::Syntax);
        assert_eq!(events.is_none(), syntax_error, "{dialect} {text:?}");
        for event in events.iter().flatten() {
            assert!(
                event.offset < text.len() && text.is_char_boundary(event.offset),
                "{dialect} {text:?}: {event:?}"
            );
        }
        if dialect == Dialect::Json
            && checked.passed()
            && let Ok(expected) = serde_json::from_str::<Value>(text)
        {
            let events = events.expect("events for text that passes");
            assert_eq!(fold(&events), expected, "{text:?}: {events:?}");
        }
    }
});
