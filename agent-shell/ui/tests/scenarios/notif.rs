//! "Notifications" column in real GTK under headless Wayland: filters, cards of all subtypes,
//! hostile text, keyboard, "Open chat", the collapsed strip on a narrow window, window title, toast.

use std::time::Duration;

use controllers::notif_types::EventKind;
use controllers::notif_view::Selection;
use controllers::testkit::evil_strings;
use fake_agentd::events_db::NewEvent;
use gtk4 as gtk;
use gtk4::prelude::*;
use ui::shell::ColumnId;

use crate::ctx::{Ctx, Scenario, WAIT, settle};
use crate::tree::{Rig, find_all, open, wait_loaded};

pub fn scenarios() -> Vec<(&'static str, Scenario)> {
    vec![
        ("ui_notifications_filters_and_cards", filters_and_cards),
        ("ui_notifications_all_eleven_subtypes", all_subtypes),
        ("ui_notifications_text_is_literal", text_is_literal),
        ("ui_notifications_keyboard", keyboard),
        ("ui_notifications_open_chat", open_chat),
        ("ui_notifications_narrow_window_strip", narrow_strip),
        ("ui_notifications_window_title", window_title),
        ("ui_notifications_toast", toast),
    ]
}

/// Feed from the mock fixtures: 13 events, 11 unread (one per subtype).
fn loaded(ctx: &Ctx) -> Result<Rig, String> {
    let rig = open(ctx)?;
    wait_loaded(&rig)?;
    settle("feed from fixtures", WAIT, || {
        rig.shell.notif_view().cards.len() == 13
    })?;
    Ok(rig)
}

fn buttons(container: &gtk::Box) -> Vec<gtk::Button> {
    find_all::<gtk::Button>(container.upcast_ref())
}

fn click(button: &gtk::Button) {
    button.emit_clicked();
}

fn chip(container: &gtk::Box, index: usize) -> Result<gtk::Button, String> {
    buttons(container)
        .into_iter()
        .nth(index)
        .ok_or_else(|| format!("no filter button #{index}"))
}

fn card_labels(rig: &Rig) -> Vec<gtk::Label> {
    find_all::<gtk::Label>(rig.shell.notif_widgets().list().upcast_ref())
}

fn filters_and_cards(ctx: &Ctx) -> Result<(), String> {
    let rig = loaded(ctx)?;
    let w = rig.shell.notif_widgets();
    let cards = find_all::<gtk::Box>(w.list().upcast_ref())
        .into_iter()
        .filter(|b| b.has_css_class("notif-card"))
        .count();
    if cards != 13 {
        return Err(format!("cards in the list {cards}, expected 13"));
    }
    let top = buttons(&w.filters());
    if top.len() != 4 {
        return Err(format!("filters 'All' + 3 kinds: {}", top.len()));
    }
    click(&top[1]);
    settle("AI kind selected", WAIT, || {
        rig.shell.notif_view().selection == Selection::Kind(EventKind::Ai)
    })?;
    settle("AI subfilters", WAIT, || buttons(&w.subs()).len() == 4)?;
    click(&chip(&w.subs(), 1)?);
    settle(
        "subtype watch selected",
        WAIT,
        || matches!(&rig.shell.notif_view().selection, Selection::Sub(_, s) if s == "watch"),
    )?;
    click(&chip(&w.subs(), 1)?);
    settle("a second click cleared the subtype", WAIT, || {
        rig.shell.notif_view().selection == Selection::Kind(EventKind::Ai)
    })?;
    click(&chip(&w.filters(), 0)?);
    settle("'All'", WAIT, || {
        rig.shell.notif_view().selection == Selection::All
    })
}

fn all_subtypes(ctx: &Ctx) -> Result<(), String> {
    let rig = loaded(ctx)?;
    let w = rig.shell.notif_widgets();
    let kinds = [
        (
            EventKind::Ai,
            1usize,
            ["agent", "watch", "claude", "local"].as_slice(),
        ),
        (EventKind::Task, 2, ["sched", "deleg", "done"].as_slice()),
        (EventKind::Msg, 3, ["sys", "wa", "tg", "mail"].as_slice()),
    ];
    let mut seen = 0;
    for (kind, index, subs) in kinds {
        click(&chip(&w.filters(), index)?);
        settle("kind selected", WAIT, || {
            rig.shell.notif_view().selection == Selection::Kind(kind)
        })?;
        settle("kind subfilters", WAIT, || {
            buttons(&w.subs()).len() == subs.len()
        })?;
        for (i, sub) in subs.iter().enumerate() {
            click(&chip(&w.subs(), i)?);
            settle(&format!("card of subtype {sub}"), WAIT, || {
                let v = rig.shell.notif_view();
                !v.cards.is_empty() && v.cards.iter().all(|c| c.sub == *sub)
            })?;
            let v = rig.shell.notif_view();
            if !v
                .kinds
                .iter()
                .any(|k| k.subs.iter().any(|s| s.sub == *sub && s.badge.count >= 1))
            {
                return Err(format!("subtype {sub} has no counter"));
            }
            seen += 1;
            click(&chip(&w.subs(), i)?);
            settle("subtype cleared", WAIT, || {
                rig.shell.notif_view().selection == Selection::Kind(kind)
            })?;
        }
    }
    if seen != 11 {
        return Err(format!("subtypes checked {seen}, expected 11"));
    }
    Ok(())
}

fn text_is_literal(ctx: &Ctx) -> Result<(), String> {
    let rig = loaded(ctx)?;
    let evil = evil_strings();
    for (i, text) in evil.iter().enumerate() {
        let e = NewEvent {
            kind: "msg",
            sub: "sys",
            text,
            ts: rig.d.now_unix() + i as i64,
            ..NewEvent::default()
        };
        rig.d.add_event(&e).map_err(|e| e.to_string())?;
    }
    settle(
        "hostile events in the feed",
        Duration::from_secs(20),
        || rig.shell.notif_view().cards.len() == 13 + evil.len(),
    )?;
    let shown = card_labels(&rig);
    for text in &evil {
        if !shown.iter().any(|l| l.text() == text.as_str()) {
            return Err(format!(
                "text is not shown literally: {:?}",
                text.chars().take(40).collect::<String>()
            ));
        }
    }
    if let Some(l) = shown.iter().find(|l| l.uses_markup()) {
        return Err(format!("label with Pango markup: {:?}", l.text()));
    }
    Ok(())
}

fn keyboard(ctx: &Ctx) -> Result<(), String> {
    let rig = loaded(ctx)?;
    let w = rig.shell.notif_widgets();
    for b in buttons(&w.filters())
        .iter()
        .chain(buttons(&w.list()).iter())
    {
        if !b.is_focusable() || !b.can_focus() {
            return Err("a feed button does not get keyboard focus".into());
        }
    }
    let kind = chip(&w.filters(), 2)?;
    kind.grab_focus();
    if !kind.is_focus() {
        return Err("focus did not land on the 'Tasks' filter".into());
    }
    // Enter on a button fires `activate`
    kind.emit_by_name::<()>("activate", &[]);
    settle("Enter selected the 'Tasks' kind", WAIT, || {
        rig.shell.notif_view().selection == Selection::Kind(EventKind::Task)
    })?;
    settle("focus returned to the filter after a rebuild", WAIT, || {
        chip(&w.filters(), 2).is_ok_and(|b| b.is_focus())
    })
}

fn open_chat(ctx: &Ctx) -> Result<(), String> {
    let rig = loaded(ctx)?;
    let w = rig.shell.notif_widgets();
    let view = rig.shell.notif_view();
    let card = view
        .cards
        .iter()
        .find(|c| c.session_id.is_some() && c.unread)
        .ok_or("no unread card with a session")?
        .clone();
    let session = card.session_id.ok_or("no session")?;
    let button = buttons(&w.list())
        .into_iter()
        .find(|b| b.label().as_deref() == Some(card.action_label.as_str()))
        .ok_or("no 'Open chat' button")?;
    click(&button);
    settle("session opened in chat", WAIT, || {
        rig.shell.chat_view().session_id == Some(session)
    })?;
    settle("session events read", WAIT, || {
        !rig.shell
            .notif_view()
            .cards
            .iter()
            .any(|c| c.session_id == Some(session) && c.unread)
    })?;
    Ok(())
}

fn narrow_strip(ctx: &Ctx) -> Result<(), String> {
    let rig = open(ctx)?;
    rig.shell.window().set_default_size(1024, 700);
    let column = rig.shell.column(ColumnId::Notifications);
    settle("column collapsed at 1024 px", WAIT, || {
        column.has_css_class("collapsed") && rig.shell.notif_view().collapsed
    })
    .map_err(|e| {
        let widths: Vec<i32> = [
            ColumnId::Notifications,
            ColumnId::Sessions,
            ColumnId::Chat,
            ColumnId::Tools,
        ]
        .iter()
        .map(|c| rig.shell.column(*c).width())
        .collect();
        format!(
            "{e}; window {}x{}, column widths {widths:?}",
            rig.shell.window().width(),
            rig.shell.window().height()
        )
    })?;
    let w = rig.shell.notif_widgets();
    settle("kind strip", WAIT, || buttons(&w.strip()).len() == 3)?;
    if !w.strip().is_visible() {
        return Err("strip is not visible".into());
    }
    click(&chip(&w.strip(), 2)?);
    settle(
        "column expanded with the mouse on a narrow window",
        WAIT,
        || !column.has_css_class("collapsed") && !rig.shell.notif_view().collapsed,
    )?;
    if rig.shell.notif_view().selection != Selection::Kind(EventKind::Msg) {
        return Err("column did not open with the 'Messages' filter".into());
    }
    Ok(())
}

fn window_title(ctx: &Ctx) -> Result<(), String> {
    let rig = loaded(ctx)?;
    settle("title (11) Agent OS", WAIT, || {
        rig.shell.window().title().as_deref() == Some("(11) Agent OS")
    })?;
    ctx.rt
        .block_on(
            rig.http
                .read_events(&core_client::events::EventsRead::default()),
        )
        .map_err(|e| e.to_string())?;
    settle("title without a number", WAIT, || {
        rig.shell.window().title().as_deref() == Some("Agent OS")
    })
}

fn toast(ctx: &Ctx) -> Result<(), String> {
    let rig = loaded(ctx)?;
    let quiet = ctx
        .rt
        .block_on(rig.http.create_session("Quiet", None, None))
        .map_err(|e| e.to_string())?["id"]
        .as_i64()
        .ok_or("no session id")?;
    let active = rig
        .shell
        .tree_view()
        .rows
        .iter()
        .find(|r| r.name == "Release 1.2")
        .map(|r| r.node);
    let Some(controllers::tree_types::NodeId::Session(active)) = active else {
        return Err("no session 'Release 1.2'".into());
    };
    crate::chat::select_session(&rig, "Release 1.2")?;
    let _ = active;
    ctx.rt
        .block_on(rig.http.send_message(
            quiet,
            "00000000-0000-4000-8000-0000000000aa",
            "hello",
            &[],
        ))
        .map_err(|e| e.to_string())?;
    let toast = rig.shell.toast();
    settle(
        "toast for a reply in an inactive session",
        Duration::from_secs(10),
        || toast.root.is_visible(),
    )?;
    settle(
        "toast disappeared after 4 s",
        Duration::from_secs(8),
        || !toast.root.is_visible(),
    )
}
