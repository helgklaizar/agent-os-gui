//! A4-06 / A4-08: the observer in the agent action flow: a dangerous command and an unavailable observer.

mod common;

use common::{Env, agent_log, finished, row};
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn observer_dangerous_cmd_flow() {
    let mut e = Env::start().await;
    let (_, s) = e.model_session().await;
    assert_eq!(
        e.admin
            .put("/permissions/global/0/shell", json!({"mode": "allow"}))
            .await
            .0,
        200
    );
    e.say(s, "c1", "#scn:action_danger clean up").await;
    let (agent, tool) = finished(&e, s).await;
    assert!(agent[1].contains("blocked_by_observer"), "{agent:?}");
    assert_eq!(tool.unwrap()["meta"]["code"], "blocked_by_observer");
    assert!(
        e.messages(s).await.iter().all(|m| m["role"] != "system"),
        "a block has no card"
    );
    assert_eq!(
        agent_log(&mut e, "shell").await,
        vec![row("shell", "deny", "observer_blocked")]
    );
    let f: Vec<(String, String, String)> = e
        .inspect(|c| {
            c.prepare("SELECT kind, verdict, ref_table FROM observer_findings")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect()
        })
        .await;
    assert_eq!(
        f,
        vec![(
            "dangerous_cmd".into(),
            "blocked".into(),
            "actions_log".into()
        )]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn observer_unavailable_dangerous_fail_closed() {
    let e = Env::start().await;
    let (_, s) = e.model_session().await;
    for a in ["delete", "shell"] {
        assert_eq!(
            e.admin
                .put(
                    &format!("/permissions/global/0/{a}"),
                    json!({"mode": "allow"})
                )
                .await
                .0,
            200
        );
    }
    assert_eq!(
        e.admin
            .post("/_test/observer", json!({"available": false}))
            .await
            .0,
        200
    );
    e.say(s, "c1", "#scn:action_delete notes").await;
    let (id, card) = e.wait_card(s).await;
    let r = &card["meta"]["request"];
    assert_eq!(
        (
            r["source"].as_str(),
            r["level"].as_str(),
            r["verdict"].as_str()
        ),
        (Some("observer"), Some("warn"), Some("asked")),
        "{r}"
    );
    assert!(
        r["detail"]
            .as_str()
            .unwrap()
            .contains("observer unavailable")
    );
    assert_eq!(e.decide(id, json!({"decision": "deny"})).await.0, 200);
    finished(&e, s).await;
    // A safe read passes without a card while the observer is unavailable.
    let (_, s2) = e.model_session().await;
    e.say(s2, "c2", "#scn:action_read file").await;
    let (agent, _) = finished(&e, s2).await;
    assert!(agent[1].contains("executor_unavailable"), "{agent:?}");
    assert!(e.messages(s2).await.iter().all(|m| m["role"] != "system"));
}
