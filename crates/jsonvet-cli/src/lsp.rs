//! `jsonvet lsp`: a language server on standard input and output. It checks each open document
//! as a run checks the file at its path, offers the safe fixes as code actions, and formats as
//! `jsonvet fmt` does, all of a document or a range of it.

use std::cell::Cell;
use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use jsonvet::{Config, RelatedKind, Severity};
use lsp_server::{Connection, ErrorCode, Message, Notification, Request, Response};
use serde_json::{Value, json};

use crate::args::ConfigSource;
use crate::settings::{Resolver, Settings};
use crate::{editorconfig, format, frame, messages, rules, sarif};

/// The code action kind of the action that applies every safe fix, which editors can run on save.
const FIX_ALL: &str = "source.fixAll.jsonvet";

/// Run the server until the client says exit: exit status 0 after a shutdown request, else 1,
/// as the protocol asks.
pub(crate) fn run() -> Result<u8, String> {
    let (connection, threads) = Connection::stdio();
    let clean = serve(&connection)?;
    drop(connection);
    threads.join().map_err(|e| e.to_string())?;
    Ok(u8::from(!clean))
}

/// Answer the client on `connection` until it says exit or goes: whether it asked to shut down
/// first.
fn serve(connection: &Connection) -> Result<bool, String> {
    let (id, params) = connection.initialize_start().map_err(|e| e.to_string())?;
    // UTF-8 positions save a count per position, so take them when the client offers them.
    let utf8 = params
        .pointer("/capabilities/general/positionEncodings")
        .and_then(Value::as_array)
        .is_some_and(|encodings| encodings.iter().any(|e| e == "utf-8"));
    let capabilities = json!({
        "positionEncoding": if utf8 { "utf-8" } else { "utf-16" },
        // Full: every change is checked against the whole text anyway.
        "textDocumentSync": { "openClose": true, "change": 1 },
        "codeActionProvider": { "codeActionKinds": ["quickfix", FIX_ALL] },
        "documentFormattingProvider": true,
        "documentRangeFormattingProvider": true,
    });
    let result = json!({
        "capabilities": capabilities,
        "serverInfo": { "name": "jsonvet", "version": env!("CARGO_PKG_VERSION") },
    });
    connection
        .initialize_finish(id, result)
        .map_err(|e| e.to_string())?;
    let client = Client {
        utf8,
        versioned: params
            .pointer("/capabilities/workspace/workspaceEdit/documentChanges")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    };
    let send = |message: Message| connection.sender.send(message).map_err(|e| e.to_string());
    let mut documents = HashMap::new();
    let mut shut_down = false;
    for message in &connection.receiver {
        match message {
            Message::Request(request) if request.method == "shutdown" => {
                shut_down = true;
                send(Message::Response(Response::new_ok(request.id, Value::Null)))?;
            }
            // After shutdown the protocol allows only `exit`: refuse requests, drop the rest.
            Message::Request(request) if shut_down => {
                send(Message::Response(Response::new_err(
                    request.id,
                    ErrorCode::InvalidRequest as i32,
                    "the server is shutting down".to_owned(),
                )))?;
            }
            Message::Request(request) => {
                send(Message::Response(answer(&request, &documents, client)))?;
            }
            Message::Notification(note) if note.method == "exit" => return Ok(shut_down),
            Message::Notification(_) if shut_down => {}
            Message::Notification(note) => {
                if let Some(published) = update(&note, &mut documents, utf8) {
                    send(Message::Notification(published))?;
                }
            }
            Message::Response(_) => {}
        }
    }
    // The client went without saying exit.
    Ok(false)
}

/// What the client can take, from its capabilities.
#[derive(Clone, Copy)]
struct Client {
    /// Positions in UTF-8 bytes, not UTF-16 units.
    utf8: bool,
    /// Workspace edits that name the version of the document they edit.
    versioned: bool,
}

/// An open document: its text and the client's version of it, if it gave one.
struct Document {
    text: String,
    version: Option<Value>,
}

/// Keep `documents` in step with an open, change or close, and return the document's new
/// diagnostics.
fn update(
    note: &Notification,
    documents: &mut HashMap<String, Document>,
    utf8: bool,
) -> Option<Notification> {
    let params = &note.params;
    let uri = params.pointer("/textDocument/uri")?.as_str()?.to_owned();
    let version = params
        .pointer("/textDocument/version")
        .filter(|v| !v.is_null())
        .cloned();
    let diagnostics = match note.method.as_str() {
        "textDocument/didOpen" | "textDocument/didChange" => {
            let text = match note.method.as_str() {
                "textDocument/didOpen" => params.pointer("/textDocument/text"),
                // Full sync: the last change holds the whole text.
                _ => params
                    .get("contentChanges")
                    .and_then(Value::as_array)
                    .and_then(|changes| changes.last())
                    .and_then(|change| change.get("text")),
            }?
            .as_str()?
            .to_owned();
            let path = path_of(&uri)?;
            let diagnostics = diagnostics(&uri, &path, &text, utf8);
            let document = Document {
                text,
                version: version.clone(),
            };
            documents.insert(uri.clone(), document);
            diagnostics
        }
        "textDocument/didClose" => {
            documents.remove(&uri);
            Vec::new()
        }
        _ => return None,
    };
    let mut published = json!({ "uri": uri, "diagnostics": diagnostics });
    // Left out, not null, where the client gave none, as on a close.
    if let Some(version) = version {
        published["version"] = version;
    }
    Some(Notification::new(
        "textDocument/publishDiagnostics".to_owned(),
        published,
    ))
}

/// The response to a request other than shutdown.
fn answer(request: &Request, documents: &HashMap<String, Document>, client: Client) -> Response {
    let utf8 = client.utf8;
    let params = &request.params;
    let document = params
        .pointer("/textDocument/uri")
        .and_then(Value::as_str)
        .and_then(|uri| Some((uri, documents.get(uri)?, path_of(uri)?)));
    let result = match (request.method.as_str(), document) {
        ("textDocument/codeAction", Some((uri, document, path))) => {
            let lines = Lines::new(&document.text, utf8);
            let range = lines.range(params.get("range"));
            let only = params.pointer("/context/only").and_then(Value::as_array);
            let wants = |kind: &str| {
                only.is_none_or(|only| {
                    only.iter().filter_map(Value::as_str).any(|wanted| {
                        kind == wanted
                            || kind
                                .strip_prefix(wanted)
                                .is_some_and(|rest| rest.starts_with('.'))
                    })
                })
            };
            // A client that takes it gets the version, so it drops edits to a newer text.
            let version = document.version.as_ref().filter(|_| client.versioned);
            code_actions((uri, version), &path, &lines, range, &wants)
        }
        ("textDocument/formatting", Some((_, document, path))) => {
            let lines = Lines::new(&document.text, utf8);
            formatting(&path, &lines, None)
        }
        ("textDocument/rangeFormatting", Some((_, document, path))) => {
            let lines = Lines::new(&document.text, utf8);
            let range = lines.range(params.get("range"));
            formatting(&path, &lines, Some(range))
        }
        // A document the server does not have, or one that is not a file, gets nothing.
        (
            "textDocument/codeAction" | "textDocument/formatting" | "textDocument/rangeFormatting",
            None,
        ) => Value::Null,
        (method, _) => {
            return Response::new_err(
                request.id.clone(),
                ErrorCode::MethodNotFound as i32,
                format!("unknown method {method}"),
            );
        }
    };
    Response::new_ok(request.id.clone(), result)
}

/// The diagnostics for `text`, checked as the file at `path` is. A config file or schema that
/// stops a run is one error at the start, as nothing else can be checked.
fn diagnostics(uri: &str, path: &Path, text: &str, utf8: bool) -> Vec<Value> {
    let lines = Lines::new(text, utf8);
    let (report, schema_errors) = match crate::check_file(path, text) {
        Ok(checked) => checked,
        Err(error) => {
            return vec![json!({
                "range": lines.span(0..0),
                "severity": 1,
                "source": "jsonvet",
                "message": error,
            })];
        }
    };
    let mut diagnostics = Vec::new();
    for problem in report.problems() {
        let id = problem.rule().id();
        let mut diagnostic = json!({
            "range": lines.span(problem.offset()..problem.end()),
            "severity": if problem.severity() == Severity::Error { 1 } else { 2 },
            "code": id,
            "codeDescription": { "href": rules::help_uri(id) },
            "source": "jsonvet",
            "message": messages::problem(problem),
        });
        if let Some(related) = problem.related() {
            let label = match related.kind {
                RelatedKind::FirstOccurrence => "related.first-copy",
                RelatedKind::Opened => "related.opened-here",
                // NOTE: `RelatedKind` is non-exhaustive; `Start` is the only other kind so far.
                _ => "related.starts-here",
            };
            let (start, end) = frame::one_char(&report, related.offset);
            diagnostic["relatedInformation"] = json!([{
                "location": { "uri": uri, "range": lines.span(start..end) },
                "message": messages::text(label),
            }]);
        }
        diagnostics.push(diagnostic);
    }
    for (offset, message, _) in schema_errors {
        let (start, end) = frame::one_char(&report, offset);
        diagnostics.push(json!({
            "range": lines.span(start..end),
            "severity": 1,
            "code": sarif::SCHEMA,
            "codeDescription": { "href": rules::help_uri(sarif::SCHEMA) },
            "source": "jsonvet",
            "message": message,
        }));
    }
    diagnostics
}

/// The checking config for the file at `path`, as a run picks it.
fn config_for(path: &Path) -> Result<Config, String> {
    let mut resolver = Resolver::new(&ConfigSource::Search, Settings::default())?;
    Ok(resolver.for_file(path)?.config_for(&path.to_string_lossy()))
}

/// A quick fix for each safe fix with a problem in `range`, and one action for them all.
fn code_actions(
    (uri, version): (&str, Option<&Value>),
    path: &Path,
    lines: &Lines<'_>,
    range: Range<usize>,
    wants: &impl Fn(&str) -> bool,
) -> Value {
    let text = lines.text;
    let Ok(config) = config_for(path) else {
        return json!([]);
    };
    let Ok(all) = jsonvet::try_fix(text, &config, false) else {
        return json!([]);
    };
    let action = |title: String, kind: &str, fixed: &str| {
        let edits: Vec<Value> = edits(text, fixed)
            .into_iter()
            .map(|(span, new)| json!({ "range": lines.span(span), "newText": new }))
            .collect();
        let edit = match version {
            Some(version) => json!({ "documentChanges": [{
                "textDocument": { "uri": uri, "version": version },
                "edits": edits,
            }]}),
            None => json!({ "changes": { uri: edits } }),
        };
        json!({ "title": title, "kind": kind, "edit": edit })
    };
    let mut actions = Vec::new();
    if wants("quickfix") {
        let report = jsonvet::check(text, &config);
        for fix in &all.fixes {
            // The fix's problem may start after it, as a trailing comma's is the bracket after
            // it: the nearest problem of its rule from the fix on, else the fix's own place.
            let end = report
                .problems()
                .iter()
                .find(|p| p.rule() == fix.rule && p.offset() >= fix.offset)
                .map_or(fix.offset, jsonvet::Problem::end);
            // Inclusive at both ends, so a caret that touches the problem counts.
            if fix.offset > range.end || end < range.start {
                continue;
            }
            let Ok(one) = jsonvet::try_fix_at(text, &config, false, Some(&[fix.offset])) else {
                continue;
            };
            let id = format!("lsp.fix.{}", fix.rule.id());
            let title = match messages::text(&id) {
                "" => messages::fill("lsp.fix", &[("rule", &fix.rule.id())]),
                title => title.to_owned(),
            };
            actions.push(action(title, "quickfix", &one.text));
        }
    }
    if wants(FIX_ALL) {
        actions.push(action(
            messages::text("lsp.fix-all").to_owned(),
            FIX_ALL,
            &all.text,
        ));
    }
    Value::Array(actions)
}

/// The edits that format the document as `jsonvet fmt` would, or with `range`, only those
/// inside it, so the text outside stays as it is. `null` when the text cannot be formatted,
/// such as for a syntax error.
fn formatting(path: &Path, lines: &Lines<'_>, range: Option<Range<usize>>) -> Value {
    let text = lines.text;
    let resolver = match Resolver::new(&ConfigSource::Search, Settings::default()) {
        Ok(resolver) => Mutex::new(resolver),
        Err(_) => return Value::Null,
    };
    let Ok((config, options)) = format::setup(path, &resolver, &editorconfig::Cache::default())
    else {
        return Value::Null;
    };
    let Ok(formatted) = jsonvet::format(text, &config, &options) else {
        return Value::Null;
    };
    let mut edits = edits(text, &formatted);
    if let Some(range) = range {
        let same = |edits: &[(Range<usize>, &str)]| {
            jsonvet::format(&apply(text, edits), &config, &options).is_ok_and(|f| f == formatted)
        };
        edits = within(text, edits, &range, same);
    }
    edits
        .into_iter()
        .map(|(span, new)| json!({ "range": lines.span(span), "newText": new }))
        .collect()
}

/// The `edits` to `text` inside `range` that `same` says lead where all of them do. Some alone
/// could change the document, such as one that drops a comma the formatter moves past a comment
/// when the edit that adds it back is outside the range. Then only those that change white space
/// alone, if `same` takes them, else none.
fn within<'n>(
    text: &str,
    mut edits: Vec<(Range<usize>, &'n str)>,
    range: &Range<usize>,
    same: impl Fn(&[(Range<usize>, &'n str)]) -> bool,
) -> Vec<(Range<usize>, &'n str)> {
    // An insertion at the end of the range goes on the next line, which is not selected.
    edits.retain(|(span, _)| {
        range.start <= span.start
            && (span.end < range.end
                || span.end == range.end && (span.start < span.end || span.end == text.len()))
    });
    if !same(&edits) {
        edits.retain(|(span, new)| text[span.clone()].trim().is_empty() && new.trim().is_empty());
        if !same(&edits) {
            edits.clear();
        }
    }
    edits
}

/// `text` with `edits`, which are in order and do not overlap.
fn apply(text: &str, edits: &[(Range<usize>, &str)]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for (span, new) in edits {
        out.push_str(&text[at..span.start]);
        out.push_str(new);
        at = span.end;
    }
    out.push_str(&text[at..]);
    out
}

/// How far ahead [`edits`] looks for the next token both texts share. The formatter changes
/// white space, and a few tokens: commas, quotes and a comment it moves past a colon.
const LOOKAHEAD: usize = 16;

/// The edits that turn `old` into `new`, in order, none overlapping, each as small as it can
/// be: a byte range of `old` and the text from `new` for it. The texts are split into tokens,
/// each of `{}[],:` and each run of other characters but white space, and the tokens they share
/// are kept, so a change in white space between two tokens is an edit of its own.
fn edits<'n>(old: &'n str, new: &'n str) -> Vec<(Range<usize>, &'n str)> {
    let (olds, news) = (tokens(old), tokens(new));
    let mut edits = Vec::new();
    let (mut i, mut j, mut old_at, mut new_at) = (0, 0, 0, 0);
    loop {
        let token = |tokens: &[Range<usize>], text: &'n str, at: usize| {
            tokens.get(at).map(|range| &text[range.clone()])
        };
        // The nearest pair of equal tokens, by how many tokens are skipped to reach it, where
        // the next two tokens also agree, else where only they do: `,` and `:` are common.
        let nearest = |run: usize| {
            (0..=2 * LOOKAHEAD).find_map(|skipped| {
                (0..=skipped).find_map(|k| {
                    let (x, y) = (i + k, j + skipped - k);
                    let agree = |n| token(&olds, old, x + n) == token(&news, new, y + n);
                    (x < olds.len() && y < news.len() && (0..run).all(agree)).then_some((x, y))
                })
            })
        };
        let next = nearest(3).or_else(|| nearest(1));
        let old_end = next.map_or(old.len(), |(x, _)| olds[x].start);
        let new_end = next.map_or(new.len(), |(_, y)| news[y].start);
        let (gap, with) = (&old[old_at..old_end], &new[new_at..new_end]);
        if gap != with {
            let same = |x: &mut dyn Iterator<Item = (char, char)>| {
                x.take_while(|(c, d)| c == d)
                    .map(|(c, _)| c.len_utf8())
                    .sum::<usize>()
            };
            let head = same(&mut gap.chars().zip(with.chars()));
            let tail = same(&mut gap[head..].chars().rev().zip(with[head..].chars().rev()));
            edits.push((
                old_at + head..old_end - tail,
                &new[new_at + head..new_end - tail],
            ));
        }
        let Some((x, y)) = next else { break };
        (old_at, new_at, i, j) = (olds[x].end, news[y].end, x + 1, y + 1);
    }
    edits
}

/// The byte ranges of the tokens of `text`, as [`edits`] splits it.
fn tokens(text: &str) -> Vec<Range<usize>> {
    let mut tokens: Vec<Range<usize>> = Vec::new();
    let mut joins = false;
    for (at, c) in text.char_indices() {
        let end = at + c.len_utf8();
        if c.is_whitespace() {
            joins = false;
        } else if matches!(c, '{' | '}' | '[' | ']' | ',' | ':') {
            tokens.push(at..end);
            joins = false;
        } else if joins && let Some(last) = tokens.last_mut() {
            last.end = end;
        } else {
            tokens.push(at..end);
            joins = true;
        }
    }
    tokens
}

/// A text's lines as the protocol counts them, ended by LF, CRLF or a lone CR, to turn byte
/// offsets into positions and back: in UTF-8 bytes, or else in UTF-16 units.
struct Lines<'t> {
    text: &'t str,
    starts: Vec<usize>,
    utf8: bool,
    /// Two offsets and their UTF-16 columns, for counting on from: problems come in order, and
    /// so mostly do the places they point back to, so a long line is counted once for each.
    known: [Cell<(usize, usize)>; 2],
}

impl<'t> Lines<'t> {
    fn new(text: &'t str, utf8: bool) -> Lines<'t> {
        let bytes = text.as_bytes();
        let mut starts = vec![0];
        for (at, &byte) in bytes.iter().enumerate() {
            if byte == b'\n' || byte == b'\r' && bytes.get(at + 1) != Some(&b'\n') {
                starts.push(at + 1);
            }
        }
        // In ASCII, a UTF-16 unit is a byte.
        let utf8 = utf8 || text.is_ascii();
        let known = [Cell::new((0, 0)), Cell::new((0, 0))];
        Lines {
            text,
            starts,
            utf8,
            known,
        }
    }

    /// The position of byte `offset`, a character boundary.
    fn position(&self, offset: usize) -> Value {
        let offset = offset.min(self.text.len());
        let line = self.starts.partition_point(|&start| start <= offset) - 1;
        let start = self.starts[line];
        let character = if self.utf8 {
            offset - start
        } else {
            // The known place nearest before `offset` on its line, else the line's start; the
            // other one is kept.
            let near = self
                .known
                .iter()
                .filter(|k| (start..=offset).contains(&k.get().0));
            let (slot, (from, units)) = match near.max_by_key(|k| k.get().0) {
                Some(known) => (known, known.get()),
                None => (
                    self.known
                        .iter()
                        .min_by_key(|k| k.get().0)
                        .unwrap_or(&self.known[0]),
                    (start, 0),
                ),
            };
            let units = units
                + self
                    .text
                    .get(from..offset)
                    .map_or(0, |s| s.encode_utf16().count());
            slot.set((offset, units));
            units
        };
        json!({ "line": line, "character": character })
    }

    /// The range of the bytes `span`.
    fn span(&self, span: Range<usize>) -> Value {
        json!({ "start": self.position(span.start), "end": self.position(span.end) })
    }

    /// The byte offset of a position: past the end of its line, the end of the line, and inside
    /// a character, the end of the character.
    fn offset(&self, position: Option<&Value>) -> usize {
        let number = |key| {
            position
                .and_then(|p| p.get(key))
                .and_then(Value::as_u64)
                .map_or(0, |n| usize::try_from(n).unwrap_or(usize::MAX))
        };
        let Some(&start) = self.starts.get(number("line")) else {
            return self.text.len();
        };
        let wanted = number("character");
        let mut units = 0;
        for (at, c) in self.text[start..].char_indices() {
            if units >= wanted || c == '\n' || c == '\r' {
                return start + at;
            }
            units += if self.utf8 {
                c.len_utf8()
            } else {
                c.len_utf16()
            };
        }
        self.text.len()
    }

    /// The byte range of a range.
    fn range(&self, range: Option<&Value>) -> Range<usize> {
        let start = self.offset(range.and_then(|r| r.get("start")));
        let end = self.offset(range.and_then(|r| r.get("end")));
        start..end.max(start)
    }
}

/// The path a `file:` URI names, or `None` for any other.
fn path_of(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    // A host other than none or `localhost` names another machine.
    let path = match rest.find('/')? {
        0 => rest,
        at if &rest[..at] == "localhost" => &rest[at..],
        _ => return None,
    };
    let mut bytes = Vec::with_capacity(path.len());
    let mut iter = path.bytes();
    while let Some(byte) = iter.next() {
        if byte == b'%' {
            let digit = |b: Option<u8>| char::from(b?).to_digit(16);
            let (high, low) = (digit(iter.next())?, digit(iter.next())?);
            bytes.push(u8::try_from(high * 16 + low).ok()?);
        } else {
            bytes.push(byte);
        }
    }
    let path = String::from_utf8(bytes).ok()?;
    if cfg!(windows) {
        // `/c:/x` is `c:\x`.
        let path = path.strip_prefix('/').unwrap_or(&path);
        return Some(PathBuf::from(path.replace('/', "\\")));
    }
    Some(PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    use super::{Lines, apply, edits, path_of, serve, within};
    use lsp_server::{Connection, Message, Notification, Request, RequestId};
    use serde_json::{Value, json};
    use std::path::{Path, PathBuf};

    /// A folder of its own for `test`, with an empty `jsonvet.toml`, so the user's own config
    /// file never applies.
    fn folder(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jsonvet-lsp-{test}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temporary directory");
        std::fs::write(dir.join("jsonvet.toml"), "").expect("written");
        dir
    }

    fn uri(path: &Path) -> String {
        let path = path.to_string_lossy().replace('\\', "/");
        let path = path.trim_start_matches('/');
        format!("file:///{}", path.replace(' ', "%20"))
    }

    /// A client on one end of an in-memory connection, with the server on a thread at the other.
    struct Client {
        connection: Connection,
        server: std::thread::JoinHandle<Result<bool, String>>,
        next: i32,
    }

    impl Client {
        fn start(capabilities: &Value) -> (Client, Value) {
            let (client, server) = Connection::memory();
            let server = std::thread::spawn(move || serve(&server));
            let mut client = Client {
                connection: client,
                server,
                next: 0,
            };
            let result = client.request("initialize", json!({ "capabilities": capabilities }));
            client.notify("initialized", json!({}));
            (client, result)
        }

        fn request(&mut self, method: &str, params: Value) -> Value {
            self.try_request(method, params).expect("no error")
        }

        /// The result of a request, or the code of the error it got.
        fn try_request(&mut self, method: &str, params: Value) -> Result<Value, i32> {
            self.next += 1;
            let id = RequestId::from(self.next);
            let request = Request::new(id.clone(), method.to_owned(), params);
            self.connection
                .sender
                .send(Message::Request(request))
                .expect("sent");
            loop {
                match self.connection.receiver.recv().expect("a message") {
                    Message::Response(response) if response.id == id => {
                        return response.response_result.map_err(|e| e.code);
                    }
                    _ => {}
                }
            }
        }

        fn notify(&self, method: &str, params: Value) {
            let note = Notification::new(method.to_owned(), params);
            self.connection
                .sender
                .send(Message::Notification(note))
                .expect("sent");
        }

        /// The next diagnostics the server publishes.
        fn diagnostics(&self) -> Value {
            loop {
                if let Message::Notification(note) =
                    self.connection.receiver.recv().expect("a message")
                    && note.method == "textDocument/publishDiagnostics"
                {
                    return note.params;
                }
            }
        }

        fn open(&self, uri: &str, text: &str) -> Value {
            self.notify(
                "textDocument/didOpen",
                json!({ "textDocument": {
                    "uri": uri, "languageId": "json", "version": 1, "text": text,
                }}),
            );
            self.diagnostics()
        }

        fn stop(mut self) {
            assert_eq!(self.request("shutdown", Value::Null), Value::Null);
            self.notify("exit", Value::Null);
            assert_eq!(self.server.join().expect("no panic"), Ok(true));
        }
    }

    fn range(start: (u32, u32), end: (u32, u32)) -> Value {
        json!({
            "start": { "line": start.0, "character": start.1 },
            "end": { "line": end.0, "character": end.1 },
        })
    }

    #[test]
    fn diagnostics_come_on_open_and_change_and_go_on_close() {
        let dir = folder("diagnostics");
        let file = uri(&dir.join("a.json"));
        let (client, result) = Client::start(&json!({}));
        assert_eq!(result["capabilities"]["positionEncoding"], "utf-16");
        let published = client.open(&file, "{\"a\": 1, \"a\": 2}");
        assert_eq!(published["uri"], file.as_str());
        let diagnostic = &published["diagnostics"][0];
        assert_eq!(diagnostic["code"], "duplicate-key");
        assert_eq!(diagnostic["severity"], 1);
        assert_eq!(diagnostic["message"], "duplicate key $.a");
        assert_eq!(diagnostic["range"], range((0, 9), (0, 12)));
        let related = &diagnostic["relatedInformation"][0];
        assert_eq!(related["location"]["range"], range((0, 1), (0, 2)));
        assert_eq!(related["message"], "first copy");
        client.notify(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": file, "version": 2 },
                "contentChanges": [{ "text": "{\"a\": 1}" }],
            }),
        );
        let published = client.diagnostics();
        assert_eq!(published["version"], 2);
        assert_eq!(published["diagnostics"], json!([]));
        client.notify(
            "textDocument/didClose",
            json!({ "textDocument": { "uri": file } }),
        );
        assert_eq!(client.diagnostics()["diagnostics"], json!([]));
        client.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_dialect_and_rules_come_from_the_config_file() {
        let dir = folder("config");
        std::fs::write(
            dir.join("jsonvet.toml"),
            "dialects = [{ files = \"*.json\", dialect = \"json\" }]\n\
             rules = { duplicate-key = \"warning\" }\n",
        )
        .expect("written");
        let (client, _) = Client::start(&json!({}));
        let published = client.open(&uri(&dir.join("a.json")), "{\"a\": 1, \"a\": 2,}");
        let codes: Vec<_> = published["diagnostics"]
            .as_array()
            .expect("an array")
            .iter()
            .map(|d| (d["code"].clone(), d["severity"].clone()))
            .collect();
        assert_eq!(
            codes,
            [
                (json!("duplicate-key"), json!(2)),
                (json!("syntax"), json!(1))
            ]
        );
        // A config file that cannot be used is one error at the start.
        std::fs::write(dir.join("jsonvet.toml"), "rules = 1\n").expect("written");
        let published = client.open(&uri(&dir.join("b.json")), "{}");
        let diagnostic = &published["diagnostics"][0];
        assert_eq!(diagnostic["range"], range((0, 0), (0, 0)));
        assert!(
            diagnostic["message"]
                .as_str()
                .is_some_and(|m| m.contains("`rules` must be a table")),
            "{diagnostic}"
        );
        client.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_local_schema_is_checked() {
        let dir = folder("schema");
        std::fs::write(
            dir.join("s.json"),
            r#"{"properties": {"a": {"type": "string"}}}"#,
        )
        .expect("written");
        let (client, _) = Client::start(&json!({}));
        let published = client.open(
            &uri(&dir.join("a.json")),
            "{\"$schema\": \"./s.json\",\n \"a\": 1}\n",
        );
        let diagnostic = &published["diagnostics"][0];
        assert_eq!(diagnostic["code"], "schema");
        assert_eq!(diagnostic["range"], range((1, 6), (1, 7)));
        client.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn positions_count_utf16_units_or_utf8_bytes_and_every_line_ending() {
        // `😀` is two UTF-16 units and four bytes; CRLF and a lone CR each end one line.
        let dir = folder("positions");
        let text = "{\r\n\"😀\": 1,\r\"😀\": 2}";
        let file = uri(&dir.join("a.json"));
        let (client, result) = Client::start(&json!({}));
        assert_eq!(result["capabilities"]["positionEncoding"], "utf-16");
        let published = client.open(&file, text);
        let diagnostics = &published["diagnostics"];
        assert_eq!(diagnostics[0]["code"], "mixed-line-endings");
        assert_eq!(diagnostics[0]["range"], range((1, 8), (2, 0)));
        assert_eq!(diagnostics[1]["range"], range((2, 0), (2, 4)));
        client.stop();
        let capabilities = json!({ "general": { "positionEncodings": ["utf-16", "utf-8"] } });
        let (client, result) = Client::start(&capabilities);
        assert_eq!(result["capabilities"]["positionEncoding"], "utf-8");
        let published = client.open(&file, text);
        let diagnostics = &published["diagnostics"];
        assert_eq!(diagnostics[0]["range"], range((1, 10), (2, 0)));
        assert_eq!(diagnostics[1]["range"], range((2, 0), (2, 6)));
        client.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn columns_counted_on_from_a_known_place_match_a_count_from_the_line_start() {
        let text = "é😀a\né😀a😀é\r😀a";
        let lines = Lines::new(text, false);
        let offsets: Vec<usize> = text.char_indices().map(|(at, _)| at).collect();
        // Forward, back and forward again, as problems and the places they name come.
        for &offset in offsets.iter().chain(offsets.iter().rev()).chain(&offsets) {
            let start = text[..offset].rfind(['\n', '\r']).map_or(0, |at| at + 1);
            let units = text[start..offset].encode_utf16().count();
            assert_eq!(lines.position(offset)["character"], units, "at {offset}");
        }
    }

    #[test]
    fn positions_turn_back_into_offsets() {
        let text = "a😀b\r\nc\rd";
        let lines = Lines::new(text, false);
        let at = |line: u32, character: u32| {
            lines.offset(Some(&json!({ "line": line, "character": character })))
        };
        assert_eq!(at(0, 3), 5);
        // Inside `😀`: the end of it.
        assert_eq!(at(0, 2), 5);
        // Past the end of a line: its end, before the line ending.
        assert_eq!(at(0, 99), 6);
        assert_eq!((at(1, 0), at(2, 0), at(3, 0)), (8, 10, text.len()));
        let lines = Lines::new(text, true);
        assert_eq!(lines.offset(Some(&json!({ "line": 0, "character": 5 }))), 5);
    }

    #[test]
    fn code_actions_offer_each_safe_fix_and_all_of_them() {
        let dir = folder("actions");
        let file = uri(&dir.join("a.jsonl"));
        let (mut client, _) = Client::start(&json!({}));
        // A byte order mark, and a missing final newline.
        let text = "\u{feff}{\"a\": 1}\n{\"a\": 2}";
        client.open(&file, text);
        let actions = client.request(
            "textDocument/codeAction",
            json!({
                "textDocument": { "uri": file },
                "range": range((1, 0), (1, 8)),
                "context": { "diagnostics": [] },
            }),
        );
        let titles: Vec<_> = actions
            .as_array()
            .expect("an array")
            .iter()
            .map(|a| a["title"].as_str().unwrap_or_default().to_owned())
            .collect();
        assert_eq!(
            titles,
            [
                "Add the final newline",
                "Fix every problem that has a safe fix"
            ]
        );
        assert_eq!(
            actions[0]["edit"]["changes"][file.as_str()],
            json!([{ "range": range((1, 8), (1, 8)), "newText": "\n" }])
        );
        assert_eq!(
            actions[1]["edit"]["changes"][file.as_str()],
            json!([
                { "range": range((0, 0), (0, 1)), "newText": "" },
                { "range": range((1, 8), (1, 8)), "newText": "\n" },
            ])
        );
        // On save, editors ask for the fix-all kind alone.
        let actions = client.request(
            "textDocument/codeAction",
            json!({
                "textDocument": { "uri": file },
                "range": range((0, 0), (0, 0)),
                "context": { "diagnostics": [], "only": ["source.fixAll"] },
            }),
        );
        assert_eq!(actions.as_array().map(Vec::len), Some(1));
        assert_eq!(actions[0]["kind"], "source.fixAll.jsonvet");
        client.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn formatting_gives_what_fmt_gives() {
        let dir = folder("formatting");
        std::fs::write(dir.join("jsonvet.toml"), "[format]\nindent = 4\n").expect("written");
        let file = uri(&dir.join("a.json"));
        let (mut client, _) = Client::start(&json!({}));
        let text = "{\"a\":[1,2],\n\"b\":{\"c\":true}}";
        client.open(&file, text);
        let result = client.request(
            "textDocument/formatting",
            json!({
                "textDocument": { "uri": file },
                "options": { "tabSize": 2, "insertSpaces": true },
            }),
        );
        let edits = parse_edits(text, &result);
        let options = jsonvet::FormatOptions {
            indent: jsonvet::Indent::Spaces(4),
            ..jsonvet::FormatOptions::default()
        };
        let config = jsonvet::Config::new(jsonvet::Dialect::Json5);
        let expected = jsonvet::format(text, &config, &options).expect("formatted");
        assert_eq!(apply_owned(text, &edits), expected);
        // A syntax error: nothing to do.
        client.open(&file, "{");
        let result = client.request(
            "textDocument/formatting",
            json!({ "textDocument": { "uri": file }, "options": {} }),
        );
        assert_eq!(result, Value::Null);
        client.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// `result`'s text edits, as byte ranges of `text`, for UTF-16 positions.
    fn parse_edits(text: &str, result: &Value) -> Vec<(std::ops::Range<usize>, String)> {
        let lines = Lines::new(text, false);
        result
            .as_array()
            .expect("an array")
            .iter()
            .map(|edit| {
                let span = lines.range(edit.get("range"));
                (
                    span,
                    edit["newText"].as_str().unwrap_or_default().to_owned(),
                )
            })
            .collect()
    }

    fn apply_owned(text: &str, edits: &[(std::ops::Range<usize>, String)]) -> String {
        let edits: Vec<_> = edits.iter().map(|(s, n)| (s.clone(), n.as_str())).collect();
        apply(text, &edits)
    }

    #[test]
    fn range_formatting_leaves_the_rest_byte_for_byte() {
        let dir = folder("range");
        let file = uri(&dir.join("a.json"));
        let (mut client, _) = Client::start(&json!({}));
        let text = "{\n\"a\":   [1,2],\n  \"b\" :{\"c\":\n      true},\n\"d\":    3}\n";
        client.open(&file, text);
        let result = client.request(
            "textDocument/rangeFormatting",
            json!({
                "textDocument": { "uri": file },
                "range": range((2, 0), (4, 0)),
                "options": {},
            }),
        );
        let edits = parse_edits(text, &result);
        assert_ne!(edits.len(), 0);
        let formatted = apply_owned(text, &edits);
        // Lines 1, 2 and 5 are as they were.
        let start = text.find("  \"b\"").expect("line 3");
        let end = text.find("\"d\"").expect("line 5");
        assert_eq!(&formatted[..start], &text[..start]);
        assert!(formatted.ends_with(&text[end..]));
        assert_eq!(
            &formatted[start..formatted.len() - (text.len() - end)],
            "  \"b\": { \"c\": true },\n"
        );
        client.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_range_keeps_only_edits_that_leave_the_document_whole() {
        // The formatter moves the first comma before the comment: one edit adds it there and
        // one drops it after. A range with only the second keeps the white space edit alone.
        let dir = folder("whole");
        let file = uri(&dir.join("a.jsonc"));
        let (mut client, _) = Client::start(&json!({}));
        let text = "[1 // c\n, 2,\n    3]\n";
        client.open(&file, text);
        let mut format = |range| {
            let params = json!({ "textDocument": { "uri": file }, "range": range, "options": {} });
            let result = client.request("textDocument/rangeFormatting", params);
            apply_owned(text, &parse_edits(text, &result))
        };
        assert_eq!(format(range((1, 0), (3, 0))), "[1 // c\n, 2,\n  3\n]\n");
        assert_eq!(
            format(range((0, 0), (3, 0))),
            "[\n  1, // c\n  2,\n  3\n]\n"
        );
        client.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_range_drops_an_edit_that_breaks_the_document_alone() {
        // The comma moves before the comment: the second edit alone drops it.
        let text = "[1 // c\n, 2,\n    3]\n";
        let edits = vec![(2..2, ","), (8..9, " "), (14..16, "")];
        let config = jsonvet::Config::new(jsonvet::Dialect::Jsonc);
        let options = jsonvet::FormatOptions::default();
        let formatted = jsonvet::format(text, &config, &options).expect("formatted");
        let same = |edits: &[(std::ops::Range<usize>, &str)]| {
            jsonvet::format(&apply(text, edits), &config, &options).is_ok_and(|f| f == formatted)
        };
        assert_eq!(
            within(text, edits.clone(), &(8..text.len()), same),
            [(14..16, "")]
        );
        assert_eq!(within(text, edits.clone(), &(0..text.len()), same), edits);
        // Nothing left that leads there: nothing.
        assert_eq!(within(text, edits, &(8..10), same), []);
    }

    #[test]
    fn edits_are_small_and_rebuild_the_new_text() {
        let cases = [
            ("{\"a\":1}", "{ \"a\": 1 }\n"),
            ("[1,2,]", "[1, 2]\n"),
            ("{'a': 1}", "{a: 1}\n"),
            ("", "x"),
            ("x", ""),
            ("é  ü", "é ü"),
        ];
        for (old, new) in cases {
            assert_eq!(apply(old, &edits(old, new)), new, "{old:?} to {new:?}");
        }
        assert_eq!(edits("[1,2]", "[1, 2]"), [(3..3, " ")]);
        assert_eq!(edits("é  ü", "é ü"), [(3..4, "")]);
    }

    #[test]
    fn file_uris_become_paths() {
        if cfg!(windows) {
            assert_eq!(
                path_of("file:///c%3A/a%20b/x.json"),
                Some(PathBuf::from("c:\\a b\\x.json"))
            );
        } else {
            assert_eq!(
                path_of("file:///a%20b/%C3%A9.json"),
                Some(PathBuf::from("/a b/é.json"))
            );
            assert_eq!(
                path_of("file://localhost/x.json"),
                Some(PathBuf::from("/x.json"))
            );
        }
        assert_eq!(path_of("untitled:Untitled-1"), None);
        assert_eq!(path_of("file:///%zz"), None);
        // A sign is no hex digit, though `from_str_radix` takes one.
        assert_eq!(path_of("file:///a%+f.json"), None);
        // Another host is a network share, which is not a local path.
        assert_eq!(path_of("file://server/share/x.json"), None);
        let local = path_of("file:///x.json").expect("a path");
        assert_eq!(path_of("file:///x.json?a=1#b"), Some(local.clone()));
        assert_eq!(path_of("file:///x.json#b"), Some(local));
    }

    #[test]
    fn an_unknown_request_is_an_error_and_exit_without_shutdown_fails() {
        let (client, _) = Client::start(&json!({}));
        let request = Request::new(RequestId::from(7), "jsonvet/unknown".to_owned(), json!({}));
        client
            .connection
            .sender
            .send(Message::Request(request))
            .expect("sent");
        let Message::Response(response) = client.connection.receiver.recv().expect("a message")
        else {
            panic!("not a response");
        };
        assert_eq!(response.response_result.map_err(|e| e.code), Err(-32601));
        client.notify("exit", Value::Null);
        assert_eq!(client.server.join().expect("no panic"), Ok(false));
    }

    #[test]
    fn requests_after_shutdown_are_refused_and_exit_then_succeeds() {
        let (mut client, _) = Client::start(&json!({}));
        assert_eq!(client.request("shutdown", Value::Null), Value::Null);
        client.notify("$/cancelRequest", json!({ "id": 1 }));
        client.notify(
            "textDocument/didClose",
            json!({ "textDocument": { "uri": "file:///a.json" } }),
        );
        let params = json!({ "textDocument": { "uri": "file:///a.json" }, "options": {} });
        assert_eq!(
            client.try_request("textDocument/formatting", params),
            Err(-32600)
        );
        client.notify("exit", Value::Null);
        assert_eq!(client.server.join().expect("no panic"), Ok(true));
    }

    #[test]
    fn the_end_of_input_after_shutdown_without_exit_fails() {
        let (mut client, _) = Client::start(&json!({}));
        assert_eq!(client.request("shutdown", Value::Null), Value::Null);
        let Client {
            connection, server, ..
        } = client;
        drop(connection);
        assert_eq!(server.join().expect("no panic"), Ok(false));
    }

    #[test]
    fn a_quick_fix_is_offered_anywhere_on_its_problem() {
        let dir = folder("overlap");
        let (mut client, _) = Client::start(&json!({}));
        let mut titles = |name: &str, text: &str, range: Value| {
            let file = uri(&dir.join(name));
            client.open(&file, text);
            let params = json!({
                "textDocument": { "uri": file },
                "range": range,
                "context": { "diagnostics": [], "only": ["quickfix"] },
            });
            let actions = client.request("textDocument/codeAction", params);
            actions
                .as_array()
                .expect("an array")
                .iter()
                .map(|a| a["title"].as_str().unwrap_or_default().to_owned())
                .collect::<Vec<_>>()
        };
        // The diagnostic is on `]`, the comma the fix drops is before it.
        assert_eq!(
            titles("a.jsonc", "[1,2,]\n", range((0, 5), (0, 6))),
            ["Remove the trailing comma"]
        );
        // A caret just after the byte order mark.
        assert_eq!(
            titles("b.jsonl", "\u{feff}[1]\n", range((0, 1), (0, 1))),
            ["Remove the byte order mark"]
        );
        // Not on the next line.
        assert_eq!(
            titles("c.jsonc", "[1,2,]\n\n", range((1, 0), (1, 0))),
            Vec::<String>::new()
        );
        client.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_fix_names_the_version_it_edits_when_the_client_takes_one() {
        let dir = folder("versions");
        let file = uri(&dir.join("a.jsonc"));
        let capabilities = json!({ "workspace": { "workspaceEdit": { "documentChanges": true } } });
        let (mut client, _) = Client::start(&capabilities);
        client.open(&file, "[1,]\n");
        client.notify(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": file, "version": 3 },
                "contentChanges": [{ "text": "[1,2,]\n" }],
            }),
        );
        client.diagnostics();
        let params = json!({
            "textDocument": { "uri": file },
            "range": range((0, 0), (0, 6)),
            "context": { "diagnostics": [] },
        });
        let actions = client.request("textDocument/codeAction", params);
        let edit = &actions[0]["edit"];
        assert_eq!(edit.get("changes"), None);
        let change = &edit["documentChanges"][0];
        assert_eq!(change["textDocument"], json!({ "uri": file, "version": 3 }));
        assert_eq!(change["edits"][0]["newText"], "");
        // Closing publishes no version: the document has none any more.
        client.notify(
            "textDocument/didClose",
            json!({ "textDocument": { "uri": file } }),
        );
        assert_eq!(client.diagnostics().get("version"), None);
        client.stop();
        let _ = std::fs::remove_dir_all(dir);
    }
}
