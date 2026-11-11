//! `jsonvet lsp`: a language server on standard input and output. It checks each open document
//! as a run checks the file at its path, offers the safe fixes as code actions, and formats as
//! `jsonvet fmt` does, all of a document or a range of it.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fmt::Write as _;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use jsonvet::{Config, Severity};
use lsp_server::{Connection, ErrorCode, Message, Notification, Request, RequestId, Response};
use serde_json::{Value, json};

use crate::args::ConfigSource;
use crate::settings::{Resolver, Settings};
use crate::{editorconfig, format, frame, messages, rules, sarif, settings};

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

/// The protocol's handshake on `connection`: the client's `initialize` parameters and whether
/// positions are in UTF-8, or `None` when it says exit or goes first.
fn initialize(connection: &Connection) -> Result<Option<(Value, bool)>, String> {
    let send = |message: Message| connection.sender.send(message).map_err(|e| e.to_string());
    // Exit is legal before initialization; lsp-server's handshake helpers reject it.
    let (id, params) = loop {
        match connection.receiver.recv() {
            Ok(Message::Request(request)) if request.method == "initialize" => {
                break (request.id, request.params);
            }
            Ok(Message::Request(request)) => send(Message::Response(Response::new_err(
                request.id,
                ErrorCode::ServerNotInitialized as i32,
                "expected initialize request".to_owned(),
            )))?,
            Ok(Message::Notification(note)) if note.method == "exit" => return Ok(None),
            Ok(Message::Notification(_)) => {}
            Err(_) => return Ok(None),
            Ok(message) => return Err(format!("expected initialize request, got {message:?}")),
        }
    };
    // UTF-8 positions save a count per position, so take them when the client offers them.
    let utf8 = params
        .pointer("/capabilities/general/positionEncodings")
        .and_then(Value::as_array)
        .is_some_and(|encodings| encodings.iter().any(|e| e == "utf-8"));
    let capabilities = json!({
        "positionEncoding": if utf8 { "utf-8" } else { "utf-16" },
        // Full: every change is checked against the whole text anyway.
        "textDocumentSync": { "openClose": true, "change": 1 },
        "codeActionProvider": { "codeActionKinds": ["quickfix", FIX_ALL], "resolveProvider": true },
        "documentFormattingProvider": true,
        "documentRangeFormattingProvider": true,
        "workspace": { "workspaceFolders": { "supported": true, "changeNotifications": true } },
    });
    let result = json!({
        "capabilities": capabilities,
        "serverInfo": { "name": "jsonvet", "version": env!("CARGO_PKG_VERSION") },
    });
    if send(Message::Response(Response::new_ok(id, result))).is_err() {
        return Ok(None);
    }
    // A client can send notifications, such as its settings, before `initialized` gets here.
    loop {
        match connection.receiver.recv() {
            Ok(Message::Notification(note)) if note.method == "initialized" => break,
            Ok(Message::Notification(note)) if note.method == "exit" => return Ok(None),
            Ok(Message::Notification(_)) => {}
            Err(_) => return Ok(None),
            Ok(message) => {
                return Err(format!(
                    "expected initialized notification, got {message:?}"
                ));
            }
        }
    }
    Ok(Some((params, utf8)))
}

/// Answer the client on `connection` until it says exit or goes: whether it asked to shut down
/// first.
fn serve(connection: &Connection) -> Result<bool, String> {
    let send = |message: Message| connection.sender.send(message).map_err(|e| e.to_string());
    let Some((params, utf8)) = initialize(connection)? else {
        return Ok(false);
    };
    let user = settings::user_config_path();
    if let Some(watch) = watch(&params, user.as_deref().and_then(Path::parent)) {
        send(Message::Request(watch))?;
    }
    let mut client = Client::from_params(&params, utf8);
    // lsp-server hands over one message at a time. A thread queues them all, so the loop sees
    // the changes already sent and checks only the last text of each document.
    let (queue_in, receiver) = std::sync::mpsc::channel();
    let input = connection.receiver.clone();
    std::thread::spawn(move || input.iter().try_for_each(|message| queue_in.send(message)));
    let mut queue = VecDeque::new();
    let mut documents = HashMap::new();
    let mut revision = 0;
    let mut shut_down = false;
    let (mut schema_paths, mut watch_revision) = (BTreeSet::new(), 0);
    while let Some(message) = queue.pop_front().or_else(|| receiver.recv().ok()) {
        queue.extend(receiver.try_iter());
        match message {
            // After shutdown the protocol allows only `exit`: refuse requests, a second shutdown
            // too, and drop the rest.
            Message::Request(request) if shut_down => {
                send(Message::Response(Response::new_err(
                    request.id,
                    ErrorCode::InvalidRequest as i32,
                    "the server is shutting down".to_owned(),
                )))?;
            }
            Message::Request(request) if request.method == "shutdown" => {
                shut_down = true;
                send(Message::Response(Response::new_ok(request.id, Value::Null)))?;
            }
            Message::Request(request) => {
                send(Message::Response(answer(&request, &documents, &client)))?;
            }
            Message::Notification(note) if note.method == "exit" => return Ok(shut_down),
            Message::Notification(_) if shut_down => {}
            // A settings file or a workspace folder changed: every open document again, unless
            // another settings change waits.
            Message::Notification(note)
                if note.method == "workspace/didChangeWatchedFiles"
                    || note.method == "workspace/didChangeWorkspaceFolders" =>
            {
                if let Some(event) = note.params.get("event") {
                    move_roots(&mut client.roots, event);
                }
                if superseded(&note, &queue) {
                    continue;
                }
                recheck_all(&mut documents, &client).try_for_each(send)?;
            }
            Message::Notification(note) => {
                let superseded = superseded(&note, &queue);
                let update = update(&note, &mut documents, &mut revision, &client, superseded);
                if let Some(published) = update {
                    send(Message::Notification(published))?;
                }
            }
            Message::Response(response) => refused(response).map_or(Ok(()), send)?,
        }
        for request in schema_watches(&params, &documents, &mut schema_paths, &mut watch_revision) {
            send(Message::Request(request))?;
        }
    }
    // The client went without saying exit.
    Ok(false)
}

/// A log message for an error answer to one of the server's own requests, the file watch
/// registrations: a refused one leaves config and schema files unwatched.
fn refused(response: Response) -> Option<Message> {
    let error = response.response_result.err()?;
    let message = messages::fill(
        "lsp.refused",
        &[("id", &response.id), ("error", &error.message)],
    );
    Some(Message::Notification(Notification::new(
        "window/logMessage".to_owned(),
        json!({ "type": 1, "message": message }),
    )))
}

/// Update the exact schema watches when open documents change the set of files they read.
fn schema_watches(
    params: &Value,
    documents: &HashMap<String, Document>,
    schema_paths: &mut BTreeSet<PathBuf>,
    revision: &mut u64,
) -> Vec<Request> {
    let mut requests = Vec::new();
    let paths = documents
        .values()
        .flat_map(|d| d.schemas.iter().cloned())
        .collect();
    if *schema_paths != paths && watches(params, "dynamicRegistration") {
        *revision += 1;
        let id = "jsonvet/schema-watch";
        if !schema_paths.is_empty() {
            requests.push(Request::new(
                    RequestId::from(format!("{id}/remove/{revision}")),
                    "client/unregisterCapability".to_owned(),
                    json!({ "unregisterations": [{ "id": id, "method": "workspace/didChangeWatchedFiles" }] }),
                ));
        }
        *schema_paths = paths;
        if !schema_paths.is_empty() {
            let relative = watches(params, "relativePatternSupport");
            let watchers = schema_paths.iter().map(|path| watcher(path, relative));
            let request = format!("{id}/add/{revision}");
            requests.push(register(id, request, &watchers.collect::<Vec<_>>()));
        }
    }
    requests
}

/// `roots` after a `workspace/didChangeWorkspaceFolders` `event`.
fn move_roots(roots: &mut Vec<PathBuf>, event: &Value) {
    let folders = |key| {
        let list = event.get(key).and_then(Value::as_array).into_iter();
        list.flatten()
            .filter_map(|folder| path_of(folder.get("uri")?.as_str()?))
            .collect::<Vec<_>>()
    };
    let removed = folders("removed");
    roots.retain(|root| !removed.contains(root));
    roots.extend(folders("added"));
}

/// The request that has the client watch the files settings come from, so that a change to one
/// checks every open document again, if the client's `initialize` `params` say it takes one: the
/// user's own config file in the folder `user` too, if the client takes a pattern
/// relative to a folder outside the workspace. `.editorconfig` is left out: only formatting reads
/// it, afresh for each request.
fn watch(params: &Value, user: Option<&Path>) -> Option<Request> {
    if !watches(params, "dynamicRegistration") {
        return None;
    }
    let mut watchers = vec![
        json!({ "globPattern": "**/jsonvet.toml" }),
        json!({ "globPattern": "**/pyproject.toml" }),
    ];
    if let Some(dir) = user
        && watches(params, "relativePatternSupport")
    {
        watchers.push(watcher(&dir.join("jsonvet.toml"), true));
    }
    let id = "jsonvet/watch";
    Some(register(id, id.to_owned(), &watchers))
}

/// Whether the client's `initialize` `params` set the file-watching capability `key`.
fn watches(params: &Value, key: &str) -> bool {
    let pointer = format!("/capabilities/workspace/didChangeWatchedFiles/{key}");
    params.pointer(&pointer) == Some(&Value::Bool(true))
}

/// A watcher for the file at `path`. With `relative`, the pattern is relative to the nearest
/// existing folder above it, which also catches that folder's subfolders being created later and
/// reaches outside the workspace; else it is the exact path.
fn watcher(path: &Path, relative: bool) -> Value {
    let base = path.ancestors().skip(1).find(|base| base.is_dir());
    let pattern = match base.filter(|_| relative) {
        Some(base) => {
            let pattern = exact_glob(path.strip_prefix(base).unwrap_or(path));
            json!({ "baseUri": uri_of(base), "pattern": pattern })
        }
        None => json!(exact_glob(path)),
    };
    json!({ "globPattern": pattern })
}

/// The request, with id `request`, that registers `watchers` as the file watch `id`.
fn register(id: &str, request: String, watchers: &[Value]) -> Request {
    Request::new(
        RequestId::from(request),
        "client/registerCapability".to_owned(),
        json!({ "registrations": [{
            "id": id,
            "method": "workspace/didChangeWatchedFiles",
            "registerOptions": { "watchers": watchers },
        }]}),
    )
}

/// A literal path as a glob: schema names may contain glob metacharacters.
fn exact_glob(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .chars()
        .fold(String::new(), |mut glob, c| {
            if matches!(c, '*' | '?' | '[' | ']' | '{' | '}') {
                glob.push('[');
                glob.push(c);
                glob.push(']');
            } else {
                glob.push(c);
            }
            glob
        })
}

/// Whether a message waiting in `queue` makes the check `note` asks for moot: a later change or
/// close of the same document, or a later change to the watched files, which checks every
/// document again.
fn superseded(note: &Notification, queue: &VecDeque<Message>) -> bool {
    let uri = |note: &Notification| note.params.pointer("/textDocument/uri").cloned();
    queue.iter().any(|message| {
        let Message::Notification(later) = message else {
            return false;
        };
        match (note.method.as_str(), later.method.as_str()) {
            (
                "textDocument/didOpen"
                | "textDocument/didChange"
                | "workspace/didChangeWatchedFiles",
                "workspace/didChangeWatchedFiles",
            ) => true,
            (
                "textDocument/didOpen" | "textDocument/didChange",
                "textDocument/didChange" | "textDocument/didClose",
            ) => uri(note) == uri(later),
            _ => false,
        }
    })
}

/// What the client can take, from its capabilities, and the workspace folders it opened.
struct Client {
    /// The workspace folders, as paths: none when the client named none.
    roots: Vec<PathBuf>,
    /// Positions in UTF-8 bytes, not UTF-16 units.
    utf8: bool,
    /// Workspace edits that name the version of the document they edit.
    versioned: bool,
    /// Code action data is preserved and the client resolves edits when it needs them.
    resolve: bool,
}

impl Client {
    /// The client that sent the `initialize` `params`, with positions in UTF-8 bytes if `utf8`.
    fn from_params(params: &Value, utf8: bool) -> Self {
        // Each document's config search stops at the workspace folder holding it, as a run's
        // stops at its working directory.
        let mut roots: Vec<PathBuf> = params
            .get("workspaceFolders")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|folder| path_of(folder.get("uri")?.as_str()?))
            .collect();
        if roots.is_empty() {
            roots.extend(
                params
                    .get("rootUri")
                    .and_then(Value::as_str)
                    .and_then(path_of),
            );
        }
        let flag = |pointer| params.pointer(pointer).and_then(Value::as_bool) == Some(true);
        Self {
            roots,
            utf8,
            resolve: flag("/capabilities/textDocument/codeAction/dataSupport")
                && params
                    .pointer("/capabilities/textDocument/codeAction/resolveSupport/properties")
                    .and_then(Value::as_array)
                    .is_some_and(|properties| properties.iter().any(|p| p == "edit")),
            versioned: flag("/capabilities/workspace/workspaceEdit/documentChanges"),
        }
    }
}

/// Checks every open document again, after a settings file or workspace folder changed, and
/// gives the diagnostics to publish for each.
fn recheck_all<'a>(
    documents: &'a mut HashMap<String, Document>,
    client: &'a Client,
) -> impl Iterator<Item = Message> + 'a {
    documents.iter_mut().filter_map(|(uri, document)| {
        let path = path_of(uri)?;
        let (diagnostics, schemas) = diagnostics(uri, &path, &document.text, client);
        document.schemas = schemas;
        let version = document.version.clone();
        Some(Message::Notification(publish(uri, version, diagnostics)))
    })
}

/// An open document: its text and the client's version of it, if it gave one.
struct Document {
    text: String,
    version: Option<Value>,
    /// Unlike client versions, this does not restart when a file is reopened.
    revision: u64,
    schemas: BTreeSet<PathBuf>,
}

/// Keep `documents` in step with an open, change or close, and return the document's new
/// diagnostics, unless `superseded` says a later change or close makes them moot.
fn update(
    note: &Notification,
    documents: &mut HashMap<String, Document>,
    revision: &mut u64,
    client: &Client,
    superseded: bool,
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
            let (diagnostics, schemas) = if superseded {
                // Keep the last check's schemas, so their watches stay until the next check.
                let schemas = documents.get(&uri).map(|document| document.schemas.clone());
                (Vec::new(), schemas.unwrap_or_default())
            } else {
                diagnostics(&uri, &path, &text, client)
            };
            *revision = revision.checked_add(1)?;
            let document = Document {
                text,
                version: version.clone(),
                revision: *revision,
                schemas,
            };
            documents.insert(uri.clone(), document);
            if superseded {
                return None;
            }
            diagnostics
        }
        "textDocument/didClose" => {
            documents.remove(&uri);
            Vec::new()
        }
        _ => return None,
    };
    Some(publish(&uri, version, diagnostics))
}

/// The notification that publishes `diagnostics` for the document `uri` at `version`.
fn publish(uri: &str, version: Option<Value>, diagnostics: Vec<Value>) -> Notification {
    let mut published = json!({ "uri": uri, "diagnostics": Value::Array(diagnostics) });
    // Left out, not null, where the client gave none, as on a close.
    if let Some(version) = version {
        published["version"] = version;
    }
    Notification::new("textDocument/publishDiagnostics".to_owned(), published)
}

/// The response to a request other than shutdown.
fn answer(request: &Request, documents: &HashMap<String, Document>, client: &Client) -> Response {
    if request.method == "codeAction/resolve" {
        return resolve_action(request, documents, client);
    }
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
            code_actions((uri, document), &path, &lines, range, &wants, client)
        }
        ("textDocument/formatting", Some((_, document, path))) => {
            let lines = Lines::new(&document.text, utf8);
            formatting(&path, &lines, None, &client.roots)
        }
        ("textDocument/rangeFormatting", Some((_, document, path))) => {
            let lines = Lines::new(&document.text, utf8);
            let range = lines.range(params.get("range"));
            formatting(&path, &lines, Some(range), &client.roots)
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
fn diagnostics(
    uri: &str,
    path: &Path,
    text: &str,
    client: &Client,
) -> (Vec<Value>, BTreeSet<PathBuf>) {
    let lines = Lines::new(text, client.utf8);
    let schemas = RefCell::new(BTreeSet::new());
    let checked = resolver(path, &client.roots).and_then(|r| {
        crate::check_file(path, text, r, &|path| {
            schemas.borrow_mut().insert(settings::lexical(path));
        })
    });
    let schemas = schemas.into_inner();
    let (report, schema_errors) = match checked {
        Ok(checked) => checked,
        Err(error) => {
            return (
                vec![json!({
                    "range": lines.span(0..0),
                    "severity": 1,
                    "source": "jsonvet",
                    "message": error,
                })],
                schemas,
            );
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
            let (_, label) = messages::RELATED[messages::related(related.kind)];
            let (start, end) = frame::one_char(&report, related.offset);
            diagnostic["relatedInformation"] = json!([{
                "location": { "uri": uri, "range": lines.span(start..end) },
                "message": messages::text(label),
            }]);
        }
        diagnostics.push(diagnostic);
    }
    for (offset, message, summary) in schema_errors {
        let (start, end) = frame::one_char(&report, offset);
        let mut diagnostic = json!({
            "range": lines.span(start..end),
            "severity": if summary { 3 } else { 1 },
            "source": "jsonvet",
            "message": message,
        });
        if !summary {
            diagnostic["code"] = json!(sarif::SCHEMA);
            diagnostic["codeDescription"] = json!({ "href": rules::help_uri(sarif::SCHEMA) });
        }
        diagnostics.push(diagnostic);
    }
    (diagnostics, schemas)
}

/// A resolver for the file at `path` that stops its config search at the workspace folder
/// holding it, the deepest where folders nest, or at a nearer `.git`. A file outside them all
/// stops at its own folder. With no folders, every file is outside them: the editor's working directory, often
/// `/` or the home folder, would let a config file far above apply.
fn resolver(path: &Path, roots: &[PathBuf]) -> Result<Resolver, String> {
    let root = roots
        .iter()
        .filter(|root| path.starts_with(root))
        .max_by_key(|root| root.as_os_str().len())
        .map(PathBuf::as_path)
        .or(path.parent());
    match root {
        Some(root) => Resolver::workspace(root),
        None => Resolver::new(&ConfigSource::Search, Settings::default()),
    }
}

/// The checking config for the file at `path`, as a run picks it.
fn config_for(path: &Path, roots: &[PathBuf]) -> Result<Config, String> {
    let mut resolver = resolver(path, roots)?;
    Ok(resolver.for_file(path)?.config_for(&path.to_string_lossy()))
}

/// A quick fix for each safe fix with a problem in `range`, and one action for them all.
fn code_actions(
    (uri, document): (&str, &Document),
    path: &Path,
    lines: &Lines<'_>,
    range: Range<usize>,
    wants: &impl Fn(&str) -> bool,
    client: &Client,
) -> Value {
    let text = lines.text;
    let Ok(config) = config_for(path, &client.roots) else {
        return json!([]);
    };
    let Ok(all) = jsonvet::try_fix(text, &config, false) else {
        return json!([]);
    };
    let target = (uri, document.version.as_ref().filter(|_| client.versioned));
    let action = |title: String, kind: &str, offset: Option<usize>, edit: Option<Value>| {
        let mut action = json!({ "title": title, "kind": kind });
        if let Some(edit) = edit {
            action["edit"] = edit;
        } else {
            action["data"] = json!({ "uri": uri, "revision": document.revision });
            if let Some(offset) = offset {
                action["data"]["offset"] = json!(offset);
            }
        }
        action
    };
    let mut actions = Vec::new();
    if wants("quickfix") {
        let report = jsonvet::check(text, &config);
        let in_range = |fix: &&jsonvet::Fix| {
            // The fix's problem may start after it, as a trailing comma's is the bracket after
            // it: the nearest problem of its rule from the fix on, else the fix's own place.
            let problems = report.problems();
            let from = problems.partition_point(|p| p.offset() < fix.offset);
            let end = problems[from..]
                .iter()
                .find(|p| p.rule() == fix.rule)
                .map_or(fix.offset, jsonvet::Problem::end);
            // Inclusive at both ends, so a caret that touches the problem counts.
            fix.offset <= range.end && end >= range.start
        };
        for fix in all.fixes.iter().filter(in_range) {
            let id = format!("lsp.fix.{}", fix.rule.id());
            let title = match messages::text(&id) {
                "" => messages::fill("lsp.fix", &[("rule", &fix.rule.id())]),
                title => title.to_owned(),
            };
            actions.push(action(
                title,
                "quickfix",
                Some(fix.offset),
                (!client.resolve)
                    .then(|| workspace_edits(target, lines, fix.edits.iter().cloned())),
            ));
        }
    }
    if wants(FIX_ALL) {
        actions.push(action(
            messages::text("lsp.fix-all").to_owned(),
            FIX_ALL,
            None,
            (!client.resolve).then(|| workspace_edit(target, lines, &all.text)),
        ));
    }
    Value::Array(actions)
}

/// Resolve only actions for the same open document revision that produced their offsets.
fn resolve_action(
    request: &Request,
    documents: &HashMap<String, Document>,
    client: &Client,
) -> Response {
    let params = &request.params;
    let data = params.get("data").and_then(|data| {
        params.get("title")?.as_str()?;
        let kind = params.get("kind")?.as_str()?;
        let uri = data.get("uri")?.as_str()?;
        let path = path_of(uri)?;
        let revision = data.get("revision")?.as_u64()?;
        let only = match (kind, data.get("offset")) {
            (FIX_ALL, None | Some(Value::Null)) => None,
            ("quickfix", Some(offset)) => Some([usize::try_from(offset.as_u64()?).ok()?]),
            _ => return None,
        };
        Some((uri, path, revision, only))
    });
    let Some((uri, path, revision, only)) = data else {
        return Response::new_err(
            request.id.clone(),
            ErrorCode::InvalidParams as i32,
            "invalid code action data".to_owned(),
        );
    };
    let Some(document) = documents.get(uri).filter(|d| revision == d.revision) else {
        return Response::new_err(
            request.id.clone(),
            ErrorCode::ContentModified as i32,
            "the document changed or was closed".to_owned(),
        );
    };
    let fixed = config_for(&path, &client.roots).and_then(|config| {
        jsonvet::try_fix_at(
            &document.text,
            &config,
            false,
            only.as_ref().map(<[usize; 1]>::as_slice),
        )
        .map_err(|_| "the code action is no longer available".to_owned())
    });
    let fixed = match fixed {
        Ok(fixed) => fixed,
        Err(message) => {
            return Response::new_err(request.id.clone(), ErrorCode::RequestFailed as i32, message);
        }
    };
    let lines = Lines::new(&document.text, client.utf8);
    let mut action = params.clone();
    action["edit"] = workspace_edit(
        (uri, document.version.as_ref().filter(|_| client.versioned)),
        &lines,
        &fixed.text,
    );
    Response::new_ok(request.id.clone(), action)
}

fn workspace_edit((uri, version): (&str, Option<&Value>), lines: &Lines<'_>, fixed: &str) -> Value {
    workspace_edits((uri, version), lines, edits(lines.text, fixed))
}

fn workspace_edits<'n>(
    (uri, version): (&str, Option<&Value>),
    lines: &Lines<'_>,
    edits: impl IntoIterator<Item = (Range<usize>, &'n str)>,
) -> Value {
    let edits: Vec<Value> = edits
        .into_iter()
        .map(|(span, new)| json!({ "range": lines.span(span), "newText": new }))
        .collect();
    match version {
        Some(version) => json!({ "documentChanges": [{
            "textDocument": { "uri": uri, "version": version }, "edits": edits,
        }]}),
        None => json!({ "changes": { uri: edits } }),
    }
}

/// The edits that format the document as `jsonvet fmt` would, or with `range`, only those
/// inside it, so the text outside stays as it is. `null` when the text cannot be formatted,
/// such as for a syntax error.
fn formatting(
    path: &Path,
    lines: &Lines<'_>,
    range: Option<Range<usize>>,
    roots: &[PathBuf],
) -> Value {
    let text = lines.text;
    let resolver = match resolver(path, roots) {
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
                    // Past the end of both, the tokens agree as `None`s: stop before that.
                    (x < olds.len() && (0..run).all(agree)).then_some((x, y))
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
    #[cfg(test)]
    TOKENIZED_BYTES.set(TOKENIZED_BYTES.get() + text.len());
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

#[cfg(test)]
thread_local! {
    static TOKENIZED_BYTES: Cell<usize> = const { Cell::new(0) };
}

/// A text's lines as the protocol counts them, ended by LF, CRLF or a lone CR, to turn byte
/// offsets into positions and back: in UTF-8 bytes, or else in UTF-16 units.
struct Lines<'t> {
    text: &'t str,
    starts: Vec<usize>,
    utf8: bool,
    /// Nearby offsets count on from these columns; checkpoints bound distant lookups.
    known: [Cell<(usize, usize)>; 2],
    checkpoints: RefCell<Vec<(usize, usize)>>,
    #[cfg(test)]
    counted: Cell<usize>,
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
            checkpoints: RefCell::new(Vec::new()),
            #[cfg(test)]
            counted: Cell::new(0),
        }
    }

    fn count(&self, from: usize, to: usize) -> usize {
        #[cfg(test)]
        self.counted.set(self.counted.get() + to.abs_diff(from));
        self.text
            .get(from..to)
            .map_or(0, |s| s.encode_utf16().count())
    }

    fn prefix(&self, offset: usize) -> usize {
        let mut checkpoints = self.checkpoints.borrow_mut();
        if checkpoints.is_empty() {
            checkpoints.push((0, 0));
        }
        while let Some(&(at, before)) = checkpoints.last().filter(|(at, _)| at + 4096 <= offset) {
            let mut next = at + 4096;
            while !self.text.is_char_boundary(next) {
                next += 1;
            }
            checkpoints.push((next, before + self.count(at, next)));
        }
        let (at, before) = checkpoints[checkpoints.partition_point(|&(at, _)| at <= offset) - 1];
        before + self.count(at, offset)
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
            let units = if offset - from <= 4096 {
                units + self.count(from, offset)
            } else {
                self.prefix(offset) - self.prefix(start)
            };
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

/// The `file:` URI of the absolute path `path`, as [`path_of`] reads it back.
fn uri_of(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    let mut uri = "file://".to_owned();
    // `c:/x` is `/c:/x`.
    if !path.starts_with('/') {
        uri.push('/');
    }
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            uri.push(char::from(byte));
        } else {
            let _ = write!(uri, "%{byte:02X}");
        }
    }
    uri
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
    use super::{Lines, apply, edits, path_of, serve, uri_of, watch, within};
    use lsp_server::{Connection, Message, Notification, Request, RequestId};
    use serde_json::{Value, json};
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    #[test]
    fn eager_code_actions_do_not_diff_the_document_for_each_fix() {
        let dir = folder("eager-actions-scaling");
        std::fs::write(
            dir.join("jsonvet.toml"),
            "dialects = [{ files = \"*.json\", dialect = \"json\" }]\n",
        )
        .expect("strict dialect");
        let path = dir.join("a.json");
        let uri = uri(&path);
        let client = super::Client {
            roots: vec![dir.clone()],
            utf8: false,
            versioned: false,
            resolve: false,
        };
        for n in [100, 1_000] {
            let text = format!("[{}]", vec!["[0,]"; n].join(","));
            let document = super::Document {
                text,
                version: None,
                revision: 1,
                schemas: BTreeSet::new(),
            };
            let lines = Lines::new(&document.text, false);
            super::TOKENIZED_BYTES.set(0);
            let actions = super::code_actions(
                (&uri, &document),
                &path,
                &lines,
                0..document.text.len(),
                &|_| true,
                &client,
            );
            assert_eq!(actions.as_array().map(Vec::len), Some(n + 1));
            eprintln!(
                "{n} eager fixes: {} bytes tokenized",
                super::TOKENIZED_BYTES.get()
            );
            let bound = 3 * document.text.len();
            assert!(
                super::TOKENIZED_BYTES.get() <= bound,
                "n={n}: {} > {bound}",
                super::TOKENIZED_BYTES.get()
            );
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn eager_edits_match_individual_core_fixes() {
        let dir = folder("eager-action-edits");
        std::fs::write(dir.join("jsonvet.toml"), "dialects = [{ files = \"*.json\", dialect = \"json\" }, { files = \"*.jsonl\", dialect = \"jsonl\" }]\n").expect("dialects");
        for (name, text) in [
            ("a.json", "\u{feff}[[1,],\r\n[2,]\n]"),
            ("a.jsonl", "\u{feff}{\"é😀\":1}\r\n{\"a\":2}"),
            (
                "a.json5",
                "{a:1,\n// jsonvet-ignore duplicate-key\na:2,\r\nb:0}\n",
            ),
        ] {
            let path = dir.join(name);
            let uri = uri(&path);
            let config = super::config_for(&path, std::slice::from_ref(&dir)).expect("config");
            let expected = jsonvet::try_fix(text, &config, false).expect("fixes");
            for utf8 in [false, true] {
                for versioned in [false, true] {
                    let client = super::Client {
                        roots: vec![dir.clone()],
                        utf8,
                        versioned,
                        resolve: false,
                    };
                    let document = super::Document {
                        text: text.to_owned(),
                        version: Some(json!(7)),
                        revision: 1,
                        schemas: BTreeSet::new(),
                    };
                    let lines = Lines::new(text, utf8);
                    let actions = super::code_actions(
                        (&uri, &document),
                        &path,
                        &lines,
                        0..text.len(),
                        &|kind| kind == "quickfix",
                        &client,
                    );
                    let actions = actions.as_array().expect("actions");
                    assert_eq!(actions.len(), expected.fixes.len());
                    for (action, fix) in actions.iter().zip(&expected.fixes) {
                        let edits = if versioned {
                            &action["edit"]["documentChanges"][0]["edits"]
                        } else {
                            &action["edit"]["changes"][&uri]
                        };
                        let edits = parse_edits_in(text, edits, utf8);
                        let expected =
                            jsonvet::try_fix_at(text, &config, false, Some(&[fix.offset]))
                                .expect("one fix");
                        assert_eq!(
                            apply_owned(text, &edits),
                            expected.text,
                            "{name} {:?}",
                            fix.rule
                        );
                    }
                }
            }
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn alternating_related_positions_scan_at_most_a_chunk_each() {
        let text = "é😀a".repeat(10_000);
        let lines = Lines::new(&text, false);
        for _ in 0..1_000 {
            for offset in [text.len(), 7 * 2_000, 7 * 9_000] {
                assert_eq!(
                    lines.position(offset),
                    json!({"line": 0, "character": offset / 7 * 4})
                );
            }
        }
        let bound = 2 * text.len() + 3_000 * 4_096;
        assert!(
            lines.counted.get() <= bound,
            "{} > {bound}",
            lines.counted.get()
        );
    }

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
        /// The notifications a request's response came after.
        skipped: Vec<String>,
    }

    /// The next message from the server, failing rather than hanging when a broken server sends
    /// none.
    fn receive(connection: &Connection) -> Message {
        connection
            .receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("a message within 10 s")
    }

    /// The server's result once its connection closes, so a server that missed `exit` ends.
    fn finish(
        connection: Connection,
        server: std::thread::JoinHandle<Result<bool, String>>,
    ) -> Result<bool, String> {
        drop(connection);
        server.join().expect("no panic")
    }

    impl Client {
        fn start(capabilities: &Value) -> (Client, Value) {
            Client::start_with(json!({ "capabilities": capabilities }))
        }

        /// A client that sends `params` with its initialize request.
        fn start_with(params: Value) -> (Client, Value) {
            let (client, server) = Connection::memory();
            let server = std::thread::spawn(move || serve(&server));
            let mut client = Client {
                connection: client,
                server,
                next: 0,
                skipped: Vec::new(),
            };
            let result = client.request("initialize", params);
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
                match receive(&self.connection) {
                    Message::Response(response) if response.id == id => {
                        return response.response_result.map_err(|e| e.code);
                    }
                    Message::Notification(note) => self.skipped.push(note.method),
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
                if let Message::Notification(note) = receive(&self.connection)
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
            assert_eq!(finish(self.connection, self.server), Ok(true));
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
        let published = client.open(&uri(&dir.join("b.json")), "[1, 2\n");
        let related = &published["diagnostics"][0]["relatedInformation"][0];
        assert_eq!(related["location"]["range"], range((0, 0), (0, 1)));
        assert_eq!(related["message"], "opened here");
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
    fn every_diagnostic_is_published() {
        let dir = folder("all-diagnostics");
        let (client, _) = Client::start(&json!({}));
        let text = "{\"a\":1,\"a\":2}\n".repeat(5_001);
        let published = client.open(&uri(&dir.join("a.jsonl")), &text);
        let diagnostics = published["diagnostics"].as_array().expect("diagnostics");
        assert_eq!(diagnostics.len(), 5_001);
        assert!(diagnostics.iter().all(|d| d["code"] == "duplicate-key"));
        std::fs::write(
            dir.join("s.json"),
            r##"{"$defs":{"root":{"type":"string"}},"$ref":"#/$defs/root"}"##,
        )
        .expect("schema");
        std::fs::write(
            dir.join("jsonvet.toml"),
            "schemas = [{ files = \"*.jsonl\", schema = \"s.json\" }]\n",
        )
        .expect("config");
        let published = client.open(&uri(&dir.join("with-schema.jsonl")), &text);
        let diagnostics = published["diagnostics"].as_array().expect("diagnostics");
        assert_eq!(diagnostics.len(), 10_002);
        assert_eq!(
            diagnostics.last().expect("last diagnostic")["code"],
            "schema"
        );
        client.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn every_quick_fix_is_listed_and_edits_are_resolved_on_demand() {
        let dir = folder("lazy-actions");
        for versioned in [false, true] {
            let (mut client, result) = Client::start(&json!({
                "textDocument": { "codeAction": {
                    "dataSupport": true, "resolveSupport": { "properties": ["edit"] },
                }},
                "workspace": { "workspaceEdit": { "documentChanges": versioned } },
            }));
            assert_eq!(
                result["capabilities"]["codeActionProvider"]["resolveProvider"],
                true
            );
            for count in [1, 1_000] {
                let text = format!("[{}0]", "[0,],".repeat(count));
                let file = uri(&dir.join("a.jsonc"));
                client.open(&file, &text);
                let actions = client.request(
                    "textDocument/codeAction",
                    json!({
                        "textDocument": { "uri": file },
                        "range": range((0, 0), (0, u32::try_from(text.len()).expect("small text"))),
                    }),
                );
                let actions = actions.as_array().expect("actions");
                assert_eq!(actions.len(), count + 1);
                assert!(actions.iter().all(|a| a.get("edit").is_none()));
                for (index, action) in actions[..count].iter().enumerate() {
                    assert_eq!(action["data"]["offset"], 3 + 5 * index);
                }
                for index in [0, count - 1, count] {
                    let resolved = client.request("codeAction/resolve", actions[index].clone());
                    assert_eq!(resolved["title"], actions[index]["title"]);
                    assert_eq!(resolved["kind"], actions[index]["kind"]);
                    assert_eq!(resolved["data"], actions[index]["data"]);
                    let edits = if versioned {
                        let edit = &resolved["edit"]["documentChanges"][0];
                        assert_eq!(edit["textDocument"], json!({ "uri": file, "version": 1 }));
                        &edit["edits"]
                    } else {
                        &resolved["edit"]["changes"][&file]
                    };
                    let edits = parse_edits(&text, edits);
                    let expected = if index == count {
                        text.replace(",]", "]")
                    } else {
                        let at = 3 + 5 * index;
                        format!("{}{}", &text[..at], &text[at + 1..])
                    };
                    assert_eq!(apply_owned(&text, &edits), expected);
                }
                let mut all = actions[count].clone();
                all["data"].as_object_mut().expect("data").remove("offset");
                let resolved = client.request("codeAction/resolve", all.clone());
                assert!(resolved.get("edit").is_some());
                all["data"]["offset"] = Value::Null;
                let resolved = client.request("codeAction/resolve", all);
                assert!(resolved.get("edit").is_some());
            }
            client.stop();
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn clients_without_edit_resolution_get_every_edit_immediately() {
        let dir = folder("eager-actions");
        for capabilities in [
            json!({}),
            json!({"textDocument":{"codeAction":{"dataSupport":true}}}),
            json!({"textDocument":{"codeAction":{"resolveSupport":{"properties":["edit"]}}}}),
            json!({"textDocument":{"codeAction":{"dataSupport":true,"resolveSupport":{"properties":["command"]}}}}),
        ] {
            let (mut client, _) = Client::start(&capabilities);
            let file = uri(&dir.join("a.jsonc"));
            let text = format!("[{}0]", "[0,],".repeat(21));
            client.open(&file, &text);
            let actions = client.request(
                "textDocument/codeAction",
                json!({
                    "textDocument": { "uri": file }, "range": range((0, 0), (0, 106)),
                }),
            );
            let actions = actions.as_array().expect("actions");
            assert_eq!(actions.len(), 22);
            assert!(
                actions
                    .iter()
                    .all(|a| a.get("edit").is_some() && a.get("data").is_none())
            );
            client.stop();
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn resolving_actions_rejects_malformed_stale_and_closed_documents() {
        let dir = folder("resolve-validation");
        let (mut client, _) = Client::start(&json!({"textDocument":{"codeAction":{
            "dataSupport":true, "resolveSupport":{"properties":["edit"]},
        }}}));
        let file = uri(&dir.join("a.jsonl"));
        client.open(&file, "\u{feff}{}");
        let actions = client.request(
            "textDocument/codeAction",
            json!({
                "textDocument": { "uri": file }, "range": range((0, 0), (0, 3)),
            }),
        );
        let action = actions[0].clone();
        for malformed in [
            Value::Null,
            json!({"title":"Fix", "kind":"quickfix", "data":{"uri":file,"revision":action["data"]["revision"],"offset":true}}),
            json!({"title":"Fix", "kind":"quickfix", "data":{"uri":file,"revision":action["data"]["revision"],"offset":null}}),
            json!({"title":"Fix", "kind":"quickfix", "data":{"uri":file,"revision":action["data"]["revision"]}}),
            json!({"title":"Fix", "kind":"quickfix", "data":{"uri":"https://example.org/a.jsonl","revision":action["data"]["revision"],"offset":0}}),
            json!({"title":"Fix", "kind":"source.fixAll.jsonvet", "data":{"uri":file,"revision":action["data"]["revision"],"offset":0}}),
        ] {
            assert_eq!(
                client.try_request("codeAction/resolve", malformed),
                Err(-32602)
            );
        }
        let mut unavailable = action.clone();
        unavailable["data"]["offset"] = json!(1);
        assert_eq!(
            client.try_request("codeAction/resolve", unavailable),
            Err(-32803)
        );
        std::fs::write(
            dir.join("jsonvet.toml"),
            "rules = { byte-order-mark = \"ignore\" }\n",
        )
        .expect("updated config");
        assert_eq!(
            client.try_request("codeAction/resolve", action.clone()),
            Err(-32803)
        );
        std::fs::write(dir.join("jsonvet.toml"), "rules = 1\n").expect("invalid config");
        assert_eq!(
            client.try_request("codeAction/resolve", action.clone()),
            Err(-32803)
        );
        std::fs::write(dir.join("jsonvet.toml"), "").expect("restored config");
        client.notify(
            "textDocument/didChange",
            json!({
                "textDocument": {"uri":file,"version":2}, "contentChanges":[{"text":"\u{feff}{}"}],
            }),
        );
        client.diagnostics();
        assert_eq!(
            client.try_request("codeAction/resolve", action.clone()),
            Err(-32801)
        );
        client.notify(
            "textDocument/didClose",
            json!({"textDocument":{"uri":file}}),
        );
        client.diagnostics();
        assert_eq!(
            client.try_request("codeAction/resolve", action.clone()),
            Err(-32801)
        );
        client.open(&file, "\u{feff}{}");
        assert_eq!(
            client.try_request("codeAction/resolve", action),
            Err(-32801)
        );
        client.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn schema_summaries_are_not_error_diagnostics() {
        let dir = folder("schema-summary");
        std::fs::write(dir.join("s.json"), r#"{"items":{"type":"string"}}"#).expect("schema");
        std::fs::write(
            dir.join("jsonvet.toml"),
            "schemas = [{ files = \"a.json\", schema = \"s.json\" }]\n",
        )
        .expect("config");
        let (client, _) = Client::start(&json!({}));
        let text = format!("[{}0]", "0,".repeat(100));
        let published = client.open(&uri(&dir.join("a.json")), &text);
        let diagnostics = published["diagnostics"].as_array().expect("diagnostics");
        assert_eq!(diagnostics.len(), 101);
        assert!(diagnostics[..100].iter().all(|d| d["severity"] == 1));
        assert_eq!(diagnostics[100]["severity"], 3);
        assert_eq!(
            diagnostics[100]["message"],
            crate::messages::text("schema.more")
        );
        assert!(diagnostics[100].get("code").is_none());
        client.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_request_the_client_refuses_is_logged() {
        let (client, _) = Client::start(&json!({}));
        let refused = lsp_server::Response::new_err(
            RequestId::from("watch".to_owned()),
            lsp_server::ErrorCode::InvalidParams as i32,
            "no file watching".to_owned(),
        );
        client
            .connection
            .sender
            .send(Message::Response(refused))
            .expect("sent");
        let logged = loop {
            if let Message::Notification(note) = receive(&client.connection)
                && note.method == "window/logMessage"
            {
                break note.params;
            }
        };
        assert_eq!(logged["type"], 1);
        let message = logged["message"].as_str().expect("message");
        assert!(
            message.contains("watch") && message.contains("no file watching"),
            "{message}"
        );
        client.stop();
    }

    #[test]
    fn requests_before_initialization_get_server_not_initialized() {
        let (server, client) = Connection::memory();
        client
            .sender
            .send(Message::Notification(Notification::new(
                "early".to_owned(),
                Value::Null,
            )))
            .expect("early notification");
        client
            .sender
            .send(Message::Request(Request::new(
                RequestId::from(1),
                "textDocument/formatting".to_owned(),
                json!({}),
            )))
            .expect("early request");
        client
            .sender
            .send(Message::Request(Request::new(
                RequestId::from(2),
                "initialize".to_owned(),
                json!({}),
            )))
            .expect("initialize");
        client
            .sender
            .send(Message::Notification(Notification::new(
                "initialized".to_owned(),
                json!({}),
            )))
            .expect("initialized");
        client
            .sender
            .send(Message::Notification(Notification::new(
                "exit".to_owned(),
                Value::Null,
            )))
            .expect("exit");
        assert!(!serve(&server).expect("initialized"));
        let Message::Response(response) = receive(&client) else {
            panic!("response")
        };
        assert_eq!(response.id, RequestId::from(1));
        assert_eq!(
            response.response_result.expect_err("not initialized").code,
            -32002
        );
        let Message::Response(response) = receive(&client) else {
            panic!("response")
        };
        assert_eq!(response.id, RequestId::from(2));
        assert_eq!(
            response.response_result.expect("initialized")["serverInfo"]["name"],
            "jsonvet"
        );
    }

    #[test]
    fn disconnect_before_the_initialize_response_is_an_unclean_exit() {
        let (server, client) = Connection::memory();
        client
            .sender
            .send(Message::Request(Request::new(
                RequestId::from(1),
                "initialize".to_owned(),
                json!({}),
            )))
            .expect("initialize");
        drop(client);
        assert!(!serve(&server).expect("disconnected client"));
    }

    #[test]
    fn disconnect_before_initialization_is_an_unclean_exit() {
        for after_response in [false, true] {
            let (server, client) = Connection::memory();
            let serving = std::thread::spawn(move || serve(&server));
            if after_response {
                client
                    .sender
                    .send(Message::Request(Request::new(
                        RequestId::from(1),
                        "initialize".to_owned(),
                        json!({}),
                    )))
                    .expect("initialize");
                assert!(matches!(receive(&client), Message::Response(_)));
            }
            drop(client);
            assert!(
                !serving
                    .join()
                    .expect("server")
                    .expect("disconnected client")
            );
        }
    }

    #[test]
    fn exit_during_initialization_is_an_unclean_exit() {
        for after_response in [false, true] {
            let (server, client) = Connection::memory();
            let serving = std::thread::spawn(move || serve(&server));
            if after_response {
                client
                    .sender
                    .send(Message::Request(Request::new(
                        RequestId::from(1),
                        "initialize".to_owned(),
                        json!({}),
                    )))
                    .expect("initialize");
                assert!(matches!(receive(&client), Message::Response(_)));
            }
            client
                .sender
                .send(Message::Notification(Notification::new(
                    "exit".to_owned(),
                    Value::Null,
                )))
                .expect("exit");
            assert!(!serving.join().expect("server").expect("exit notification"));
        }
    }

    #[test]
    fn invalid_initialization_messages_remain_protocol_errors() {
        for (after_response, request) in [(false, false), (true, false), (true, true)] {
            let (server, client) = Connection::memory();
            let serving = std::thread::spawn(move || serve(&server));
            if after_response {
                client
                    .sender
                    .send(Message::Request(Request::new(
                        RequestId::from(1),
                        "initialize".to_owned(),
                        json!({}),
                    )))
                    .expect("initialize");
                assert!(matches!(receive(&client), Message::Response(_)));
            }
            let message = if request {
                Message::Request(Request::new(
                    RequestId::from(2),
                    "textDocument/formatting".to_owned(),
                    json!({}),
                ))
            } else {
                Message::Response(lsp_server::Response::new_ok(
                    RequestId::from(2),
                    Value::Null,
                ))
            };
            client.sender.send(message).expect("unexpected message");
            assert!(
                serving
                    .join()
                    .expect("server")
                    .expect_err("protocol error")
                    .contains(if after_response {
                        "expected initialized notification"
                    } else {
                        "expected initialize request"
                    })
            );
        }
    }

    #[test]
    fn per_file_rules_apply_to_diagnostics_and_fixes() {
        let dir = folder("per-file-rules");
        std::fs::write(
            dir.join("jsonvet.toml"),
            "rules = { byte-order-mark = \"error\" }\n\
             per-file-rules = [\n\
               { files = \"warning.json5\", byte-order-mark = \"warning\" },\n\
               { files = \"ignored.json5\", byte-order-mark = \"ignore\" },\n\
             ]\n",
        )
        .expect("written");
        let (mut client, _) = Client::start(&json!({}));
        for (name, severity) in [("error", 1), ("warning", 2), ("ignored", 0)] {
            let file = uri(&dir.join(format!("{name}.json5")));
            let published = client.open(&file, "\u{feff}{}");
            if severity == 0 {
                assert_eq!(published["diagnostics"], json!([]));
            } else {
                assert_eq!(published["diagnostics"][0]["code"], "byte-order-mark");
                assert_eq!(published["diagnostics"][0]["severity"], severity);
            }
            let actions = client.request(
                "textDocument/codeAction",
                json!({ "textDocument": { "uri": file }, "range": range((0, 0), (0, 8)) }),
            );
            assert_eq!(
                actions.as_array().expect("actions").is_empty(),
                severity == 0
            );
        }
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
        // As in a run, a JSON Lines file's `$schema` names no schema: each line is a document.
        let published = client.open(
            &uri(&dir.join("b.jsonl")),
            "{\"$schema\": \"./s.json\", \"a\": 1}\n",
        );
        assert_eq!(published["diagnostics"], json!([]));
        client.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn positions_count_utf16_units_or_utf8_bytes_and_every_line_ending() {
        // `😀` is two UTF-16 units and four bytes; CRLF and a lone CR each end one line.
        let dir = folder("positions");
        let text = "{\r\n\"😀\": 1,\r\"😀\": 2}";
        let file = uri(&dir.join("a.json"));
        let only = json!({ "general": { "positionEncodings": ["utf-16"] } });
        let (client, result) = Client::start(&only);
        assert_eq!(result["capabilities"]["positionEncoding"], "utf-16");
        client.stop();
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
        let long = format!("{}\r\n{}", "é😀a".repeat(3_000), "a😀é".repeat(3_000));
        for text in ["é😀a\né😀a😀é\r😀a", long.as_str()] {
            let lines = Lines::new(text, false);
            let offsets: Vec<usize> = text
                .char_indices()
                .map(|(at, _)| at)
                .step_by(if text.len() > 100 { 13 } else { 1 })
                .collect();
            // Forward, back and forward again, as problems and the places they name come.
            for &offset in offsets.iter().chain(offsets.iter().rev()).chain(&offsets) {
                let start = text[..offset].rfind(['\n', '\r']).map_or(0, |at| at + 1);
                let units = text[start..offset].encode_utf16().count();
                assert_eq!(lines.position(offset)["character"], units, "at {offset}");
            }
        }
    }

    #[test]
    fn positions_on_short_lines_do_not_count_unrelated_earlier_lines() {
        let tail = "a😀é";
        let text = format!("{}\n{tail}", "é".repeat(524_288));
        let lines = Lines::new(&text, false);
        assert_eq!(
            lines.position(text.len()),
            json!({ "line": 1, "character": 4 })
        );
        assert!(
            lines.counted.get() <= tail.len(),
            "{} bytes counted for a {}-byte line",
            lines.counted.get(),
            tail.len()
        );
    }

    #[test]
    fn distant_multiline_positions_match_utf16_counts_from_each_line_start() {
        let first = "é😀\r\n";
        let text = format!("{first}{}", "a😀é".repeat(3_000));
        let lines = Lines::new(&text, false);
        let second = first.len();
        let offsets = [
            text.len(),
            2,
            second + 7 * 900,
            0,
            second + 7 * 2_500,
            second,
            second + 14,
        ];
        for offset in offsets.into_iter().chain(offsets.into_iter().rev()) {
            let prefix = &text[..offset];
            let start = prefix.rfind(['\r', '\n']).map_or(0, |at| at + 1);
            let line = prefix
                .replace("\r\n", "\n")
                .replace('\r', "\n")
                .matches('\n')
                .count();
            assert_eq!(
                lines.position(offset),
                json!({
                    "line": line, "character": text[start..offset].encode_utf16().count()
                }),
                "at {offset}"
            );
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
        let actions = client.request(
            "textDocument/codeAction",
            json!({ "textDocument": { "uri": file }, "range": range((0, 1), (0, 1)),
                "context": { "only": ["quickfix"] } }),
        );
        assert_eq!(actions.as_array().expect("actions").len(), 1);
        assert_eq!(actions[0]["title"], "Remove the byte order mark");
        client.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn formatting_gives_what_fmt_gives() {
        let dir = folder("formatting");
        std::fs::write(
            dir.join("jsonvet.toml"),
            "[format]\nindent = 2\n[[per-file-format]]\nfiles = \"a.json\"\nindent = 4\n",
        )
        .expect("written");
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
        let other = uri(&dir.join("b.json"));
        client.open(&other, text);
        let result = client.request(
            "textDocument/formatting",
            json!({ "textDocument": { "uri": other }, "options": {} }),
        );
        assert_eq!(
            apply_owned(text, &parse_edits(text, &result)),
            jsonvet::format(text, &config, &jsonvet::FormatOptions::default()).expect("formatted")
        );
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
        parse_edits_in(text, result, false)
    }

    /// `result`'s text edits, as byte ranges of `text`, for UTF-8 positions with `utf8`.
    fn parse_edits_in(
        text: &str,
        result: &Value,
        utf8: bool,
    ) -> Vec<(std::ops::Range<usize>, String)> {
        let lines = Lines::new(text, utf8);
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
    fn a_range_keeps_a_deletion_that_ends_where_it_does() {
        let text = "[1,  2]\n";
        let edits = vec![(4..5, "")];
        assert_eq!(within(text, edits.clone(), &(0..5), |_| true), edits);
        // An insertion there goes on what follows the range, so it is left out.
        assert_eq!(within(text, vec![(5..5, " ")], &(0..5), |_| true), []);
    }

    #[test]
    fn edits_find_a_shared_token_up_to_twice_the_lookahead_away() {
        // `z` is 20 tokens in: anchored on it, the space added after it stays an edit of its own.
        let old = "a0 a1 a2 a3 a4 a5 a6 a7 a8 a9 a10 a11 a12 a13 a14 a15 a16 a17 a18 a19 z b";
        let at = old.find('z').expect("z");
        assert_eq!(edits(old, "z  b"), [(0..at, ""), (at + 2..at + 2, " ")]);
    }

    #[test]
    fn edits_step_past_each_shared_token_once() {
        // Every token is the same, so taking one twice would anchor on it again.
        assert_eq!(edits("1 1 1 1 1", "1 1  1 1 1"), [(4..4, " ")]);
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
    fn paths_become_file_uris_that_read_back() {
        let path = std::env::temp_dir().join("a b#%?é~.json");
        let uri = uri_of(&path);
        assert!(
            uri.starts_with("file:///") && !uri.starts_with("file:////"),
            "{uri}"
        );
        assert!(uri.ends_with("/a%20b%23%25%3F%C3%A9~.json"), "{uri}");
        assert_eq!(path_of(&uri), Some(path));
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
        let Message::Response(response) = receive(&client.connection) else {
            panic!("not a response");
        };
        assert_eq!(response.response_result.map_err(|e| e.code), Err(-32601));
        client.notify("exit", Value::Null);
        // Exit ends the server before its connection closes.
        let started = std::time::Instant::now();
        while !client.server.is_finished() {
            assert!(started.elapsed().as_secs() < 10, "the server did not exit");
            std::thread::yield_now();
        }
        assert_eq!(finish(client.connection, client.server), Ok(false));
    }

    #[test]
    fn requests_after_shutdown_are_refused_and_exit_then_succeeds() {
        let (mut client, _) = Client::start(&json!({}));
        assert_eq!(client.request("shutdown", Value::Null), Value::Null);
        assert_eq!(client.try_request("shutdown", Value::Null), Err(-32600));
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
        // Nothing is published for the close after shutdown.
        assert_eq!(client.skipped, Vec::<String>::new());
        client.notify("exit", Value::Null);
        assert_eq!(finish(client.connection, client.server), Ok(true));
    }

    #[test]
    fn the_end_of_input_after_shutdown_without_exit_fails() {
        let (mut client, _) = Client::start(&json!({}));
        assert_eq!(client.request("shutdown", Value::Null), Value::Null);
        let Client {
            connection, server, ..
        } = client;
        assert_eq!(finish(connection, server), Ok(false));
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
        // At the end of the bracket, past the line ending before it that also has a fix.
        assert_eq!(
            titles("d.jsonc", "[1,\r\n2,\n]\n", range((2, 1), (2, 1))),
            ["Remove the trailing comma"]
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

    #[test]
    fn notifications_before_initialized_are_dropped() {
        let (server, client) = Connection::memory();
        let serving = std::thread::spawn(move || serve(&server));
        let send = |message| client.sender.send(message).expect("sent");
        send(Message::Request(Request::new(
            RequestId::from(1),
            "initialize".to_owned(),
            json!({ "capabilities": {} }),
        )));
        assert!(matches!(receive(&client), Message::Response(_)));
        // VS Code sends its configuration before `initialized` can reach the server.
        send(Message::Notification(Notification::new(
            "workspace/didChangeConfiguration".to_owned(),
            json!({ "settings": {} }),
        )));
        send(Message::Notification(Notification::new(
            "initialized".to_owned(),
            json!({}),
        )));
        send(Message::Request(Request::new(
            RequestId::from(2),
            "shutdown".to_owned(),
            Value::Null,
        )));
        let Message::Response(response) = receive(&client) else {
            panic!("a response");
        };
        assert_eq!(response.id, RequestId::from(2));
        send(Message::Notification(Notification::new(
            "exit".to_owned(),
            Value::Null,
        )));
        assert_eq!(serving.join().expect("no panic"), Ok(true));
    }

    #[test]
    fn a_config_change_checks_every_open_document_again() {
        let dir = folder("watched");
        let (client, _) = Client::start(&json!({
            "workspace": { "didChangeWatchedFiles": { "dynamicRegistration": true } },
        }));
        let Message::Request(register) = receive(&client.connection) else {
            panic!("a registration request");
        };
        assert_eq!(register.method, "client/registerCapability");
        let registration = &register.params["registrations"][0];
        assert_eq!(registration["method"], "workspace/didChangeWatchedFiles");
        let watchers = &registration["registerOptions"]["watchers"];
        assert_eq!(watchers[0]["globPattern"], "**/jsonvet.toml");
        assert_eq!(watchers[1]["globPattern"], "**/pyproject.toml");
        client
            .connection
            .sender
            .send(Message::Response(lsp_server::Response::new_ok(
                register.id,
                Value::Null,
            )))
            .expect("sent");
        let files = [uri(&dir.join("a.json")), uri(&dir.join("b.json"))];
        for file in &files {
            let published = client.open(file, "{\"a\": 1, \"a\": 2}");
            assert_eq!(published["diagnostics"][0]["severity"], 1);
        }
        std::fs::write(
            dir.join("jsonvet.toml"),
            "rules = { duplicate-key = \"warning\" }\n",
        )
        .expect("written");
        client.notify(
            "workspace/didChangeWatchedFiles",
            json!({ "changes": [{ "uri": uri(&dir.join("jsonvet.toml")), "type": 2 }] }),
        );
        let mut published: Vec<_> = (0..files.len())
            .map(|_| {
                let params = client.diagnostics();
                (
                    params["uri"].as_str().unwrap_or_default().to_owned(),
                    params["version"].clone(),
                    params["diagnostics"][0]["severity"].clone(),
                )
            })
            .collect();
        published.sort_by(|a, b| a.0.cmp(&b.0));
        let expected: Vec<_> = files
            .iter()
            .map(|file| (file.clone(), json!(1), json!(2)))
            .collect();
        assert_eq!(published, expected);
        client.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_document_schemas_stay_watched_until_they_are_created_again() {
        for initially_missing in [false, true] {
            let dir = folder(if initially_missing {
                "schema-initially-missing"
            } else {
                "schema-recreated"
            });
            let name = if initially_missing {
                "nested/s.json"
            } else {
                "s.json"
            };
            let schema = dir.join(name);
            if !initially_missing {
                std::fs::write(&schema, "false").expect("schema");
            }
            let (client, _) = Client::start(&json!({ "workspace": {
                "didChangeWatchedFiles": { "dynamicRegistration": true, "relativePatternSupport": true }
            }}));
            assert!(matches!(receive(&client.connection), Message::Request(_)));
            let file = uri(&dir.join("a.json"));
            let published = client.open(&file, &json!({ "$schema": name }).to_string());
            assert_eq!(
                published["diagnostics"]
                    .as_array()
                    .expect("diagnostics")
                    .is_empty(),
                initially_missing
            );
            let Message::Request(watch) = receive(&client.connection) else {
                panic!("schema watch");
            };
            assert_eq!(watch.method, "client/registerCapability");
            assert_eq!(
                watch.params["registrations"][0]["registerOptions"]["watchers"][0]["globPattern"],
                json!({ "baseUri": uri_of(&dir), "pattern": name })
            );
            if !initially_missing {
                std::fs::remove_file(&schema).expect("schema deleted");
                client.notify(
                    "workspace/didChangeWatchedFiles",
                    json!({ "changes": [{ "uri": uri(&schema), "type": 3 }] }),
                );
                assert_eq!(client.diagnostics()["diagnostics"], json!([]));
                // A response after deletion proves the schema watch was not unregistered.
                client
                    .connection
                    .sender
                    .send(Message::Request(Request::new(
                        900.into(),
                        "textDocument/formatting".to_owned(),
                        json!({"textDocument":{"uri":file}}),
                    )))
                    .expect("request");
                assert!(
                    matches!(receive(&client.connection), Message::Response(_)),
                    "missing schemas must stay watched"
                );
            }
            std::fs::create_dir_all(schema.parent().expect("parent")).expect("schema directory");
            std::fs::write(&schema, "false").expect("schema created");
            client.notify(
                "workspace/didChangeWatchedFiles",
                json!({ "changes": [{ "uri": uri(&schema), "type": 1 }] }),
            );
            assert_eq!(client.diagnostics()["diagnostics"][0]["code"], "schema");
            client.stop();
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn missing_schema_dependencies_do_not_escape_the_document_boundary() {
        let dir = folder("missing-schema-confinement");
        let workspace = dir.join("workspace");
        std::fs::create_dir_all(&workspace).expect("workspace");
        let file = workspace.join("a.json");
        let observed = std::cell::RefCell::new(Vec::new());
        let check = |text: &str| {
            let resolver =
                super::resolver(&file, std::slice::from_ref(&workspace)).expect("settings");
            let (_, errors) = crate::check_file(&file, text, resolver, &|path| {
                observed.borrow_mut().push(path.to_owned());
            })
            .expect("checked");
            assert!(!errors.is_empty(), "the outside schema is refused");
            assert!(
                observed.borrow().is_empty(),
                "outside paths must not be watched"
            );
        };
        check(r#"{"$schema":"../missing.json"}"#);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("../missing.json", workspace.join("linked.json"))
                .expect("dangling symlink");
            check(r#"{"$schema":"linked.json"}"#);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn schema_watches_require_client_support() {
        let mut paths = std::collections::BTreeSet::from([Path::new("s.json").to_owned()]);
        let mut revision = 7;
        assert!(
            super::schema_watches(
                &json!({}),
                &std::collections::HashMap::new(),
                &mut paths,
                &mut revision
            )
            .is_empty()
        );
        assert_eq!(paths.len(), 1);
        assert_eq!(revision, 7);
    }

    #[test]
    fn schema_watches_can_name_files_outside_the_workspace() {
        let dir = folder("relative-schema-watch");
        let mut documents = std::collections::HashMap::new();
        documents.insert(
            "file:///a.json".to_owned(),
            super::Document {
                text: "{}".to_owned(),
                version: None,
                revision: 1,
                schemas: std::collections::BTreeSet::from([dir.join("s[1].json")]),
            },
        );
        let requests = super::schema_watches(
            &json!({ "capabilities": { "workspace": {
                "didChangeWatchedFiles": { "dynamicRegistration": true, "relativePatternSupport": true }
            }}}),
            &documents,
            &mut std::collections::BTreeSet::new(),
            &mut 0,
        );
        assert_eq!(
            requests[0].params["registrations"][0]["registerOptions"]["watchers"],
            json!([{ "globPattern": { "baseUri": uri_of(&dir), "pattern": "s[[]1[]].json" } }])
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_superseded_change_keeps_the_schemas_its_watches_follow() {
        let schemas = std::collections::BTreeSet::from([PathBuf::from("/s.json")]);
        let mut documents = std::collections::HashMap::new();
        documents.insert(
            "file:///a.json".to_owned(),
            super::Document {
                text: "{}".to_owned(),
                version: None,
                revision: 1,
                schemas: schemas.clone(),
            },
        );
        let change = Notification::new(
            "textDocument/didChange".to_owned(),
            json!({
                "textDocument": { "uri": "file:///a.json", "version": 2 },
                "contentChanges": [{ "text": "{\"a\": 1}" }],
            }),
        );
        let server = super::Client {
            roots: Vec::new(),
            utf8: false,
            resolve: false,
            versioned: false,
        };
        let published = super::update(&change, &mut documents, &mut 1, &server, true);
        assert!(published.is_none());
        let document = &documents["file:///a.json"];
        assert_eq!(document.text, "{\"a\": 1}");
        assert_eq!(document.schemas, schemas);
    }

    #[test]
    fn schema_watch_globs_keep_metacharacters_literal() {
        assert_eq!(
            super::exact_glob(Path::new("schemas/[draft]*?{x}.json")),
            "schemas/[[]draft[]][*][?][{]x[}].json"
        );
    }

    #[test]
    fn local_schema_edits_refresh_open_documents_and_watchers_follow_them() {
        let dir = folder("schema-watch");
        std::fs::write(
            dir.join("s.json"),
            r##"{"$defs":{"root":{"type":"string"}},"$ref":"#/$defs/root"}"##,
        )
        .expect("schema");
        std::fs::write(dir.join("other.json"), "true").expect("schema");
        std::fs::write(
            dir.join("jsonvet.toml"),
            "schemas = [{ files = \"configured.json\", schema = \"s.json\" }]\n",
        )
        .expect("config");
        let (client, _) = Client::start(&json!({ "workspace": {
            "didChangeWatchedFiles": { "dynamicRegistration": true }
        }}));
        assert!(matches!(receive(&client.connection), Message::Request(_)));
        let configured = uri(&dir.join("configured.json"));
        assert_eq!(
            client.open(&configured, "{}")["diagnostics"][0]["code"],
            "schema"
        );
        let Message::Request(register) = receive(&client.connection) else {
            panic!("schema watch");
        };
        assert_eq!(register.method, "client/registerCapability");
        let first_registration = register.id.clone();
        let watchers = &register.params["registrations"][0]["registerOptions"]["watchers"];
        assert_eq!(watchers.as_array().expect("watchers").len(), 1);
        assert_eq!(
            watchers[0]["globPattern"],
            dir.join("s.json").to_string_lossy().replace('\\', "/")
        );
        let own = uri(&dir.join("own.json"));
        assert_eq!(
            client.open(&own, r#"{"$schema":"s.json"}"#)["diagnostics"][0]["code"],
            "schema"
        );
        std::fs::write(dir.join("s.json"), "true").expect("schema edited");
        client.notify(
            "workspace/didChangeWatchedFiles",
            json!({ "changes": [{
                "uri": uri(&dir.join("s.json")), "type": 2
            }]}),
        );
        for _ in 0..2 {
            assert_eq!(client.diagnostics()["diagnostics"], json!([]));
        }
        client.notify(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": own, "version": 2 },
                "contentChanges": [{ "text": r#"{"$schema":"other.json"}"# }]
            }),
        );
        assert_eq!(client.diagnostics()["diagnostics"], json!([]));
        let Message::Request(unregister) = receive(&client.connection) else {
            panic!("unregister");
        };
        assert_eq!(unregister.method, "client/unregisterCapability");
        let Message::Request(register) = receive(&client.connection) else {
            panic!("schema watches");
        };
        assert_ne!(register.id, first_registration);
        assert_ne!(register.id, unregister.id);
        assert_eq!(
            register.params["registrations"][0]["registerOptions"]["watchers"]
                .as_array()
                .expect("watchers")
                .len(),
            2
        );
        client.notify(
            "textDocument/didClose",
            json!({ "textDocument": { "uri": own } }),
        );
        client.diagnostics();
        assert!(matches!(receive(&client.connection), Message::Request(_)));
        let Message::Request(register) = receive(&client.connection) else {
            panic!("schema watch");
        };
        assert_eq!(
            register.params["registrations"][0]["registerOptions"]["watchers"]
                .as_array()
                .expect("watchers")
                .len(),
            1
        );
        client.notify(
            "textDocument/didClose",
            json!({ "textDocument": { "uri": configured } }),
        );
        client.diagnostics();
        let Message::Request(unregister) = receive(&client.connection) else {
            panic!("unregister last schema");
        };
        assert_eq!(unregister.method, "client/unregisterCapability");
        client.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_user_config_folder_is_watched_where_the_client_takes_relative_patterns() {
        let dir = folder("user-watch");
        let watchers = |capabilities: Value, user: Option<&Path>| {
            let params = json!({ "capabilities": { "workspace": {
                "didChangeWatchedFiles": capabilities,
            }}});
            watch(&params, user).map(|request| {
                request.params["registrations"][0]["registerOptions"]["watchers"].clone()
            })
        };
        let both = json!({ "dynamicRegistration": true, "relativePatternSupport": true });
        assert_eq!(watchers(json!({}), Some(&dir)), None);
        assert_eq!(
            watchers(json!({ "relativePatternSupport": true }), Some(&dir)),
            None
        );
        let all = watchers(both.clone(), Some(&dir)).expect("a request");
        assert_eq!(
            all[2]["globPattern"],
            json!({ "baseUri": uri_of(&dir), "pattern": "jsonvet.toml" })
        );
        let shared = json!({ "dynamicRegistration": true });
        let count = |w: Option<Value>| w.and_then(|w| w.as_array().map(Vec::len));
        assert_eq!(count(watchers(shared, Some(&dir))), Some(2));
        assert_eq!(count(watchers(both.clone(), None)), Some(2));
        let missing = watchers(both, Some(&dir.join("missing"))).expect("a watch");
        assert_eq!(
            missing[2]["globPattern"],
            json!({
                "baseUri": uri_of(&dir), "pattern": "missing/jsonvet.toml"
            })
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_git_directory_above_the_workspace_does_not_widen_its_boundary() {
        let dir = folder("workspace-git-boundary");
        std::fs::create_dir_all(dir.join(".git")).expect("git directory");
        std::fs::write(
            dir.join("jsonvet.toml"),
            "rules = { duplicate-key = \"ignore\" }\n",
        )
        .expect("parent config");
        std::fs::write(dir.join("secret.json"), "false").expect("parent schema");
        let workspace = dir.join("workspace");
        std::fs::create_dir(&workspace).expect("workspace");
        let (client, _) =
            Client::start_with(json!({"rootUri": uri(&workspace), "capabilities": {}}));
        let file = uri(&workspace.join("a.json"));
        assert_eq!(
            client.open(&file, r#"{"a":1,"a":2}"#)["diagnostics"][0]["code"],
            "duplicate-key"
        );
        let published = client.open(&file, r#"{"$schema":"../secret.json"}"#);
        assert!(
            published["diagnostics"][0]["message"]
                .as_str()
                .expect("error")
                .contains("outside"),
            "{published}"
        );
        client.stop();
        // Without a workspace, an ambient repository still cannot widen the file's boundary.
        let (client, _) = Client::start(&json!({}));
        assert_eq!(
            client.open(&file, r#"{"a":1,"a":2}"#)["diagnostics"][0]["code"],
            "duplicate-key"
        );
        let published = client.open(&file, r#"{"$schema":"../secret.json"}"#);
        assert!(
            published["diagnostics"][0]["message"]
                .as_str()
                .expect("error")
                .contains("outside"),
            "{published}"
        );
        client.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_config_search_stops_at_the_workspace_folder_holding_the_file() {
        // `a` and `b` are workspace folders; the server's own working directory is elsewhere.
        let dir = folder("roots");
        let (a, b) = (dir.join("a"), dir.join("b"));
        for sub in [a.join("sub"), b.join("sub")] {
            std::fs::create_dir_all(sub).expect("a folder");
        }
        // Above both folders: never read.
        std::fs::write(
            dir.join("jsonvet.toml"),
            "rules = { duplicate-key = \"ignore\" }\n",
        )
        .expect("written");
        std::fs::write(
            a.join("jsonvet.toml"),
            "rules = { duplicate-key = \"warning\" }\n",
        )
        .expect("written");
        let text = "{\"a\": 1, \"a\": 2}";
        let severity = |published: Value| published["diagnostics"][0]["severity"].clone();
        let (client, _) = Client::start_with(json!({ "capabilities": {}, "rootUri": uri(&a) }));
        assert_eq!(severity(client.open(&uri(&a.join("sub/x.json")), text)), 2);
        client.stop();
        let folders = json!([{ "uri": uri(&a), "name": "a" }, { "uri": uri(&b), "name": "b" }]);
        let (client, _) = Client::start_with(json!({
            "capabilities": {}, "rootUri": uri(&b), "workspaceFolders": folders,
        }));
        assert_eq!(severity(client.open(&uri(&a.join("sub/x.json")), text)), 2);
        assert_eq!(severity(client.open(&uri(&b.join("sub/y.json")), text)), 1);
        client.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn workspace_folders_added_and_removed_later_move_the_search_stop() {
        let dir = folder("roots-changed");
        let (a, b) = (dir.join("a"), dir.join("b"));
        for sub in [a.join("sub"), b.join("sub")] {
            std::fs::create_dir_all(sub).expect("a folder");
        }
        for root in [&a, &b] {
            std::fs::write(
                root.join("jsonvet.toml"),
                "rules = { duplicate-key = \"warning\" }\n",
            )
            .expect("written");
        }
        let text = "{\"a\": 1, \"a\": 2}";
        let severity = |published: Value| published["diagnostics"][0]["severity"].clone();
        let folders = |path: &Path| json!([{ "uri": uri(path), "name": "f" }]);
        let (client, result) = Client::start_with(json!({
            "capabilities": {}, "workspaceFolders": folders(&a),
        }));
        assert_eq!(
            result["capabilities"]["workspace"]["workspaceFolders"]["changeNotifications"],
            true
        );
        let (x, y) = (uri(&a.join("sub/x.json")), uri(&b.join("sub/y.json")));
        assert_eq!(severity(client.open(&x, text)), 2);
        // Outside every folder, the search stops at the file's own folder.
        assert_eq!(severity(client.open(&y, text)), 1);
        client.notify(
            "workspace/didChangeWorkspaceFolders",
            json!({ "event": { "added": folders(&b), "removed": folders(&a) } }),
        );
        // Both open documents are checked again, in either order.
        let mut published: Vec<Value> = (0..2).map(|_| client.diagnostics()).collect();
        published.sort_by_key(|p| p["uri"].as_str().map(str::to_owned));
        assert_eq!(published[0]["uri"], x);
        assert_eq!(severity(published[0].clone()), 1);
        assert_eq!(severity(published[1].clone()), 2);
        client.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_settings_change_queued_behind_others_checks_once() {
        let dir = folder("coalesce-watched");
        let file = uri(&dir.join("a.json"));
        let (mut client, _) = Client::start(&json!({}));
        client.open(&file, "{}");
        let slow = format!("{{{}\"b\": 0}}", "\"a\": 1, ".repeat(2_000));
        let change = |version: i32| {
            client.notify(
                "textDocument/didChange",
                json!({
                    "textDocument": { "uri": file, "version": version },
                    "contentChanges": [{ "text": slow }],
                }),
            );
        };
        // The first change is checked alone; the rest queue up behind it.
        change(2);
        change(3);
        let settings = json!({ "changes": [{ "uri": uri(&dir.join("jsonvet.toml")), "type": 2 }] });
        client.notify("workspace/didChangeWatchedFiles", settings.clone());
        client.notify("workspace/didChangeWatchedFiles", settings);
        client.request(
            "textDocument/formatting",
            json!({ "textDocument": { "uri": file } }),
        );
        let published = client
            .skipped
            .iter()
            .filter(|method| *method == "textDocument/publishDiagnostics")
            .count();
        assert!(published <= 2, "{published} publishes");
        client.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn queued_changes_are_checked_once_at_their_last_text() {
        let dir = folder("coalesce");
        let file = uri(&dir.join("a.json"));
        let (client, _) = Client::start(&json!({}));
        client.open(&file, "{}");
        // Slow enough to check that the changes queue up behind the first.
        let slow = format!("{{{}\"b\": 0}}", "\"a\": 1, ".repeat(2_000));
        let last = 100;
        for version in 2..=last {
            let text = if version == last {
                "{\"c\": 1, \"c\": 2}"
            } else {
                &slow
            };
            client.notify(
                "textDocument/didChange",
                json!({
                    "textDocument": { "uri": file, "version": version },
                    "contentChanges": [{ "text": text }],
                }),
            );
        }
        let mut count = 0;
        let final_check = loop {
            let params = client.diagnostics();
            count += 1;
            if params["version"] == last {
                break params;
            }
        };
        assert!(count < 10, "{count} publishes");
        let diagnostics = final_check["diagnostics"].as_array().expect("diagnostics");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0]["message"], "duplicate key $.c");
        client.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn per_file_settings_apply_when_a_fix_is_resolved_and_to_formatting() {
        let dir = folder("per-file-resolve");
        std::fs::write(
            dir.join("jsonvet.toml"),
            "rules = { byte-order-mark = \"error\" }\n\
             per-file-rules = [{ files = \"ignored.json5\", byte-order-mark = \"ignore\" }]\n\
             dialects = [\n\
               { files = \"*.json\", dialect = \"json\" },\n\
               { files = \"loose.json\", dialect = \"jsonc\" },\n\
             ]\n",
        )
        .expect("written");
        let (mut client, _) = Client::start(&json!({
            "textDocument": { "codeAction": {
                "dataSupport": true, "resolveSupport": { "properties": ["edit"] },
            }},
        }));
        let (kept, ignored) = (
            uri(&dir.join("kept.json5")),
            uri(&dir.join("ignored.json5")),
        );
        client.open(&kept, "\u{feff}{}");
        client.open(&ignored, "\u{feff}{}");
        let actions = client.request(
            "textDocument/codeAction",
            json!({ "textDocument": { "uri": kept }, "range": range((0, 0), (0, 1)) }),
        );
        let fix = actions[0].clone();
        assert_eq!(fix["kind"], "quickfix");
        let resolved = client.request("codeAction/resolve", fix.clone());
        assert_eq!(
            resolved["edit"]["changes"][&kept],
            json!([{ "range": range((0, 0), (0, 1)), "newText": "" }])
        );
        // The same fix, sent for the file whose rules leave the mark alone: nothing to fix.
        let mut stale = fix;
        stale["data"]["uri"] = json!(ignored);
        stale["data"]["revision"] = json!(2);
        assert_eq!(
            client.try_request("codeAction/resolve", stale),
            Err(lsp_server::ErrorCode::RequestFailed as i32)
        );
        // A comment is a syntax error in JSON, but `loose.json` is JSONC.
        let text = "{// c\n\"a\":1}";
        let (strict, loose) = (uri(&dir.join("strict.json")), uri(&dir.join("loose.json")));
        client.open(&strict, text);
        client.open(&loose, text);
        let options = json!({ "tabSize": 2, "insertSpaces": true });
        let config = jsonvet::Config::new(jsonvet::Dialect::Jsonc);
        let expected =
            jsonvet::format(text, &config, &jsonvet::FormatOptions::default()).expect("formatted");
        for (method, extra) in [
            ("textDocument/formatting", json!({})),
            (
                "textDocument/rangeFormatting",
                json!({ "range": range((0, 0), (2, 0)) }),
            ),
        ] {
            let mut params = json!({ "textDocument": { "uri": strict }, "options": options });
            params.as_object_mut().expect("params").extend(
                extra
                    .as_object()
                    .expect("extra")
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone())),
            );
            assert_eq!(
                client.request(method, params.clone()),
                Value::Null,
                "{method}"
            );
            params["textDocument"]["uri"] = json!(loose);
            let edits = parse_edits(text, &client.request(method, params));
            assert_eq!(apply_owned(text, &edits), expected, "{method}");
        }
        client.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn requests_take_and_give_utf8_positions_on_non_ascii_text() {
        let dir = folder("utf8-requests");
        std::fs::write(
            dir.join("jsonvet.toml"),
            "dialects = [{ files = \"*.json\", dialect = \"json\" }]\n",
        )
        .expect("written");
        let (mut client, result) =
            Client::start(&json!({ "general": { "positionEncodings": ["utf-8"] } }));
        assert_eq!(result["capabilities"]["positionEncoding"], "utf-8");
        // `é` is two bytes and one UTF-16 unit, `😀` four bytes and two units: the trailing
        // comma is at byte 12 and unit 9.
        let file = uri(&dir.join("a.json"));
        let text = "{\"é😀\":[1,],\n\"b\":2}";
        client.open(&file, text);
        let actions = client.request(
            "textDocument/codeAction",
            json!({
                "textDocument": { "uri": file },
                "range": range((0, 12), (0, 12)),
                "context": { "only": ["quickfix"] },
            }),
        );
        assert_eq!(actions[0]["title"], "Remove the trailing comma");
        assert_eq!(
            actions[0]["edit"]["changes"][&file],
            json!([{ "range": range((0, 12), (0, 13)), "newText": "" }])
        );
        let text = "{\"é😀\":[1,2],\n\"b\":   \"ü\"}";
        client.open(&file, text);
        let result = client.request(
            "textDocument/formatting",
            json!({ "textDocument": { "uri": file }, "options": {} }),
        );
        let config = jsonvet::Config::new(jsonvet::Dialect::Json);
        let expected =
            jsonvet::format(text, &config, &jsonvet::FormatOptions::default()).expect("formatted");
        assert_eq!(
            apply_owned(text, &parse_edits_in(text, &result, true)),
            expected
        );
        client.stop();
        let _ = std::fs::remove_dir_all(dir);
    }
}
