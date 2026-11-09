//! Execution of tool commands: `CoreCommand` -> core client call, reply -> message to the controller.
//! No GTK and no blocking: called by a worker thread or a test. Non-tool commands return `None`.

use controllers::CoreCommand;
use controllers::permissions::{Mode, PermTable};
use controllers::tool_types::{
    Link, RunItem, TaskItem, parse_links, parse_runs, parse_task, parse_tasks,
};
use core_client::{ClientError, HttpClient};
use serde_json::{Value, json};

/// What a core reply means for the controllers.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolsReply {
    Quick {
        items: Vec<Link>,
        rev: i64,
    },
    Marks {
        session_id: i64,
        items: Vec<Link>,
        rev: i64,
    },
    /// `412`: the list changed in the core, re-read it.
    Stale {
        session_id: Option<i64>,
    },
    Perms(PermTable),
    /// `403` on writing a session permission (weakening).
    PermRejected,
    Tasks {
        session_id: i64,
        items: Vec<TaskItem>,
    },
    TaskChanged(TaskItem),
    Runs {
        task_id: i64,
        items: Vec<RunItem>,
    },
    Exported {
        path: String,
        size: i64,
    },
}

fn status_of(e: &ClientError) -> Option<u16> {
    e.api().map(|a| a.status)
}

fn mode_name(m: Mode) -> &'static str {
    m.as_str()
}

fn cfg_json(cfg: &controllers::CfgPatch) -> Value {
    let mut m = serde_json::Map::new();
    if let Some(c) = &cfg.ctx {
        m.insert("ctx".into(), json!(c));
    }
    if let Some(model) = cfg.model {
        m.insert("model".into(), json!(model));
    }
    if let Some(v) = cfg.mem {
        m.insert("mem".into(), json!(v));
    }
    if let Some(v) = cfg.auto {
        m.insert("auto".into(), json!(v));
    }
    if let Some(n) = &cfg.notif {
        m.insert("notif".into(), json!(n));
    }
    Value::Object(m)
}

/// Result: `Err` is a reason without the token or response bodies; `Ok` is replies for the controllers (possibly empty).
pub async fn execute(
    http: &HttpClient,
    command: &CoreCommand,
) -> Option<Result<Vec<ToolsReply>, String>> {
    let done = run(http, command).await?;
    Some(done.map_err(|e| e.to_string()))
}

async fn run(
    http: &HttpClient,
    command: &CoreCommand,
) -> Option<Result<Vec<ToolsReply>, ClientError>> {
    Some(match command {
        CoreCommand::FetchQuickLinks => http.quicklinks().await.map(|l| {
            vec![ToolsReply::Quick {
                items: parse_links(&Value::Array(l.items)),
                rev: l.rev,
            }]
        }),
        CoreCommand::FetchBookmarks { session_id } => http.bookmarks(*session_id).await.map(|l| {
            vec![ToolsReply::Marks {
                session_id: *session_id,
                items: parse_links(&Value::Array(l.items)),
                rev: l.rev,
            }]
        }),
        CoreCommand::PutQuickLinks { rev, items } => {
            let body: Vec<Value> = items.iter().map(Link::json).collect();
            match http.put_quicklinks(*rev, &body).await {
                Ok(l) => Ok(vec![ToolsReply::Quick {
                    items: parse_links(&Value::Array(l.items)),
                    rev: l.rev,
                }]),
                Err(e) if status_of(&e) == Some(412) => {
                    Ok(vec![ToolsReply::Stale { session_id: None }])
                }
                Err(e) => Err(e),
            }
        }
        CoreCommand::PutBookmarks {
            session_id,
            rev,
            items,
        } => {
            let body: Vec<Value> = items.iter().map(Link::json).collect();
            match http.put_bookmarks(*session_id, *rev, &body).await {
                Ok(l) => Ok(vec![ToolsReply::Marks {
                    session_id: *session_id,
                    items: parse_links(&Value::Array(l.items)),
                    rev: l.rev,
                }]),
                Err(e) if status_of(&e) == Some(412) => Ok(vec![ToolsReply::Stale {
                    session_id: Some(*session_id),
                }]),
                Err(e) => Err(e),
            }
        }
        CoreCommand::PostToolResult { session_id, result } => http
            .post_tool_result(*session_id, result.json())
            .await
            .map(|_| Vec::new()),
        CoreCommand::PatchSessionCfg { session_id, cfg } => http
            .patch_session(*session_id, json!({ "cfg": cfg_json(cfg) }))
            .await
            .map(|_| Vec::new()),
        CoreCommand::FetchPermissions => http
            .list_all_permissions()
            .await
            .map(|rows| vec![ToolsReply::Perms(PermTable::from_value(&rows))]),
        CoreCommand::PutSessionPerm {
            session_id,
            action,
            mode,
        } => {
            match http
                .put_permission("session", *session_id, action, mode_name(*mode))
                .await
            {
                Ok(_) => Ok(Vec::new()),
                Err(e) if status_of(&e) == Some(403) => Ok(vec![ToolsReply::PermRejected]),
                Err(e) => Err(e),
            }
        }
        CoreCommand::ClearSession { session_id } => {
            http.clear_session(*session_id).await.map(|()| Vec::new())
        }
        CoreCommand::ExportSession { session_id } => {
            http.export_session(*session_id).await.and_then(|v| {
                let path = v["path"].as_str().map(str::to_string);
                let size = v["size"].as_i64();
                match (path, size) {
                    (Some(path), Some(size)) => Ok(vec![ToolsReply::Exported { path, size }]),
                    _ => Err(ClientError::Protocol("export: unexpected response".into())),
                }
            })
        }
        CoreCommand::FetchTasks { session_id } => http.session_tasks(*session_id).await.map(|v| {
            vec![ToolsReply::Tasks {
                session_id: *session_id,
                items: parse_tasks(&Value::Array(v)),
            }]
        }),
        CoreCommand::CreateTask {
            session_id,
            kind,
            title,
            spec,
            cron,
        } => {
            let mut body = json!({ "kind": kind, "title": title, "spec": spec });
            if let Some(c) = cron {
                body["cron"] = json!(c);
            }
            http.create_task(*session_id, body).await.map(|v| {
                parse_task(&v)
                    .map(ToolsReply::TaskChanged)
                    .into_iter()
                    .collect()
            })
        }
        CoreCommand::SetTaskState { task_id, state } => http
            .patch_task(*task_id, json!({ "state": state }))
            .await
            .map(|v| {
                parse_task(&v)
                    .map(ToolsReply::TaskChanged)
                    .into_iter()
                    .collect()
            }),
        CoreCommand::RunTask { task_id } => http.run_task(*task_id).await.map(|_| Vec::new()),
        CoreCommand::FetchTaskRuns { task_id } => http.task_runs(*task_id, 20).await.map(|v| {
            vec![ToolsReply::Runs {
                task_id: *task_id,
                items: parse_runs(&Value::Array(v)),
            }]
        }),
        _ => return None,
    })
}
