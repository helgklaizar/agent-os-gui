//! Mock methods for tool tests: an agent command to the browser.

use serde_json::json;

use crate::{FakeAgentd, chat_msgs};

impl FakeAgentd {
    /// Core emulation: an agent command to the browser. Issues a `request_id`, sends a `browser.command` frame to `session.<id>`.
    pub fn browser_command(&self, session_id: i64, url: &str) -> String {
        let id = self.shared.tools.open(session_id, "browser");
        chat_msgs::emit(
            &self.shared,
            session_id,
            "browser.command",
            json!({ "command": "open", "url": url, "request_id": id }),
        );
        id
    }
}
