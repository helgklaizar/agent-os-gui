//! Execution of tree commands: `CoreCommand` -> core client call. No GTK and no blocking: called by a worker thread
//! or a test. Non-tree commands return `None`.

use controllers::CoreCommand;
use core_client::HttpClient;

/// Result: `Err` is a reason without the token or response bodies.
pub async fn execute(http: &HttpClient, command: &CoreCommand) -> Option<Result<(), String>> {
    let done = match command {
        CoreCommand::CreateSession {
            name,
            folder_id,
            persona_id,
        } => http
            .create_session(name, *folder_id, *persona_id)
            .await
            .map(drop),
        CoreCommand::CreateFolder {
            name,
            parent_id,
            persona_id,
        } => http
            .create_folder(name, *parent_id, *persona_id)
            .await
            .map(drop),
        CoreCommand::CreateThread { parent_id, name } => {
            http.create_thread(*parent_id, name).await.map(drop)
        }
        CoreCommand::MoveSession { id, folder_id } => {
            http.move_session(*id, *folder_id).await.map(drop)
        }
        CoreCommand::RenameSession { id, name } => http.rename_session(*id, name).await.map(drop),
        CoreCommand::RenameFolder { id, name } => http.rename_folder(*id, name).await.map(drop),
        CoreCommand::DeleteSession { id } => http.delete_session(*id).await,
        CoreCommand::DeleteFolder { id } => http.delete_folder(*id).await,
        CoreCommand::MarkRead { id } => http.mark_session_read(*id).await,
        CoreCommand::StopStream { session_id } => http.stop_session(*session_id).await,
        CoreCommand::SetSessionPersona {
            session_id,
            persona_id,
        } => http
            .patch_session(*session_id, serde_json::json!({ "persona_id": persona_id }))
            .await
            .map(drop),
        CoreCommand::SetSessionPath { session_id, path } => http
            .patch_session(*session_id, serde_json::json!({ "path": path }))
            .await
            .map(drop),
        CoreCommand::DecidePermission {
            request_id,
            decision,
            rule,
        } => crate::chat_exec::decide(http, *request_id, *decision, *rule).await,
        CoreCommand::SendMessage { .. }
        | CoreCommand::ListPersonas
        | CoreCommand::ResyncSession { .. }
        | CoreCommand::PatchTheme(_)
        | CoreCommand::Logout
        | CoreCommand::MarkEventRead { .. }
        | CoreCommand::MarkEventsRead(_)
        | CoreCommand::FetchEventSubtypes
        | CoreCommand::ResyncEvents
        | CoreCommand::FetchAbout => return None,
        // tool commands are executed by `tools_exec`
        _ => return None,
    };
    Some(done.map_err(|e| e.to_string()))
}
