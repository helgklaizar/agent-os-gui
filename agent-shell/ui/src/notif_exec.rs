//! Execution of feed commands: `CoreCommand` -> core client call. No GTK and no blocking.
//! Non-feed commands return `None`.

use controllers::CoreCommand;
use core_client::HttpClient;
use core_client::events::EventsRead;

/// Result: `Err` is a reason without the token or response bodies. `FetchEventSubtypes` and `ResyncEvents` are executed by the shell
/// (the registry reply goes to the controller, resync is the client's subscription), they are not here.
pub async fn execute(http: &HttpClient, command: &CoreCommand) -> Option<Result<(), String>> {
    let done = match command {
        CoreCommand::MarkEventRead { id } => http.read_event(*id).await,
        CoreCommand::MarkEventsRead(f) => {
            let body = EventsRead {
                ids: None,
                kind: f.kind.map(|k| k.as_str().to_string()),
                sub: f.sub.clone(),
                session_id: f.session_id,
            };
            http.read_events(&body).await.map(drop)
        }
        _ => return None,
    };
    Some(done.map_err(|e| e.to_string()))
}
