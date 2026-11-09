//! Execution of Settings dialog commands: `Cmd` -> core client call -> replies for the controller.
//! No GTK and no blocking: called by a worker thread or a test. Errors become `Reply::Failed` without response bodies.

use controllers::settings_types::{Cmd, DOCS, FileEntry, Op, Reply, SaveError};
use core_client::settings::Installed;
use core_client::{ClientError, HttpClient};
use serde_json::Value;

/// Fields from `error.details.fields`: strings or `{field, message}` objects (the real core returns objects).
fn field_names(details: &Value) -> Vec<String> {
    details
        .get("fields")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|f| match f {
                    Value::String(s) => Some(s.clone()),
                    Value::Object(_) => f.get("field").and_then(Value::as_str).map(str::to_string),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Client error -> dialog error: code, status and field paths; the core's text is not passed on.
pub fn save_error(e: &ClientError) -> SaveError {
    match e.api() {
        Some(a) => SaveError {
            status: a.status,
            code: a.code.as_str().to_string(),
            fields: field_names(&a.details),
        },
        None => SaveError::default(),
    }
}

fn failed(op: Op, e: &ClientError) -> Reply {
    Reply::Failed {
        op,
        error: save_error(e),
    }
}

fn done(op: Op, r: Result<(), ClientError>) -> Reply {
    match r {
        Ok(()) => Reply::Done(op),
        Err(e) => failed(op, &e),
    }
}

fn file_entries(list: &[Value]) -> Vec<FileEntry> {
    list.iter()
        .filter_map(|f| {
            Some(FileEntry {
                path: f.get("path")?.as_str()?.to_string(),
                size: f.get("size").and_then(Value::as_i64).unwrap_or(0),
                rev: f
                    .get("rev")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            })
        })
        .collect()
}

/// All pages and lists: a failed read gives `Failed { op: Fetch }`, the rest still arrive.
async fn fetch_all(http: &HttpClient) -> Vec<Reply> {
    let mut out = Vec::new();
    for doc in DOCS {
        out.push(fetch_page(http, doc).await);
    }
    let list = |r: Result<Vec<Value>, ClientError>, make: fn(Vec<Value>) -> Reply| match r {
        Ok(v) => make(v),
        Err(e) => failed(Op::Fetch, &e),
    };
    out.push(list(http.list_personas().await, Reply::Personas));
    out.push(list(http.list_accounts().await, Reply::Accounts));
    out.push(list(http.list_modules().await, Reply::Modules));
    out.push(list(http.list_tunnels().await, Reply::Tunnels));
    out.push(match http.list_files().await {
        Ok(v) => Reply::Files(file_entries(&v)),
        Err(e) => failed(Op::Fetch, &e),
    });
    out.push(match http.list_secrets().await {
        Ok(v) => Reply::Secrets(
            v.iter()
                .filter(|s| s.get("set").and_then(Value::as_bool).unwrap_or(false))
                .filter_map(|s| s.get("ref").and_then(Value::as_str).map(str::to_string))
                .collect(),
        ),
        Err(e) => failed(Op::Fetch, &e),
    });
    out.push(match http.list_all_permissions().await {
        Ok(rows) => Reply::Perms(rows),
        Err(e) => failed(Op::Fetch, &e),
    });
    out
}

async fn fetch_page(http: &HttpClient, doc: &str) -> Reply {
    match http.settings_page(doc).await {
        Ok(p) => Reply::Page {
            doc: doc.to_string(),
            value: p.doc,
            rev: p.rev,
            saved: false,
        },
        Err(e) => failed(Op::Fetch, &e),
    }
}

/// Execute a command; there may be several replies (`FetchAll`).
pub async fn execute(http: &HttpClient, cmd: &Cmd) -> Vec<Reply> {
    vec![match cmd {
        Cmd::FetchAll => return fetch_all(http).await,
        Cmd::FetchPage(doc) => fetch_page(http, doc).await,
        Cmd::PatchPage { doc, rev, patch } => {
            match http.patch_settings_page(doc, *rev, patch).await {
                Ok(p) => Reply::Page {
                    doc: doc.clone(),
                    value: p.doc,
                    rev: p.rev,
                    saved: true,
                },
                Err(e) => failed(Op::Patch(doc.clone()), &e),
            }
        }
        Cmd::PutSecret { reference, value } => done(
            Op::Secret(reference.clone()),
            http.put_secret(reference, value.expose()).await,
        ),
        Cmd::CreatePersona(body) => done(
            Op::Persona,
            http.create_persona(body.clone()).await.map(drop),
        ),
        Cmd::PatchPersona { id, body } => done(
            Op::Persona,
            http.patch_persona(*id, body.clone()).await.map(drop),
        ),
        Cmd::DeletePersona(id) => done(Op::Persona, http.delete_persona(*id).await),
        Cmd::CreateAccount(body) => done(
            Op::Account,
            http.create_account(body.clone()).await.map(drop),
        ),
        Cmd::DeleteAccount(id) => done(Op::Account, http.delete_account(*id).await),
        Cmd::ConnectAccount(id) => done(Op::Account, http.connect_account(*id).await.map(drop)),
        Cmd::InstallModule(body) => match http.install_module(body.clone()).await {
            Ok(Installed::Done(_)) => Reply::Done(Op::Module),
            Ok(Installed::Pending(_)) => Reply::ModulePending,
            Err(e) => failed(Op::Module, &e),
        },
        Cmd::PatchModule { id, body } => done(
            Op::Module,
            http.patch_module(id, body.clone()).await.map(drop),
        ),
        Cmd::DeleteModule(id) => done(Op::Module, http.delete_module(id).await),
        Cmd::CreateTunnel(body) => {
            done(Op::Tunnel, http.create_tunnel(body.clone()).await.map(drop))
        }
        Cmd::PatchTunnel { id, body } => done(
            Op::Tunnel,
            http.patch_tunnel(*id, body.clone()).await.map(drop),
        ),
        Cmd::DeleteTunnel(id) => done(Op::Tunnel, http.delete_tunnel(*id).await),
        Cmd::PutPermission {
            scope,
            scope_id,
            action,
            mode,
        } => done(
            Op::Permission,
            http.put_permission(scope, *scope_id, action, mode.as_str())
                .await
                .map(drop),
        ),
        Cmd::FetchFile(path) => match http.file_content(path).await {
            Ok(f) => Reply::File {
                path: path.clone(),
                text: f.text,
                rev: f.rev,
            },
            Err(e) => failed(Op::File, &e),
        },
        Cmd::PutFile { path, rev, text } => match http.put_file(path, rev, text).await {
            Ok(rev) => Reply::FileSaved {
                path: path.clone(),
                rev,
            },
            Err(e) => failed(Op::File, &e),
        },
    }]
}

/// Parsing of a `settings` frame from the bus: `{page, rev}`.
pub fn changed(data: &Value) -> Option<(String, i64)> {
    Some((
        data.get("page")?.as_str()?.to_string(),
        data.get("rev")?.as_i64()?,
    ))
}
