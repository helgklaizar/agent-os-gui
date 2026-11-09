//! Model request assembly: system text, history messages, `models` page settings.

use serde_json::Value;

use crate::{ChatContext, HistoryItem, Price, Turn, TurnRole};

/// System text: `AGENT.md`, then the style, language and signature of the effective persona.
#[must_use]
pub(crate) fn system_text(ctx: &ChatContext) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(md) = &ctx.agent_md {
        parts.push(md.trim().to_string());
    }
    if !ctx.facts.is_empty() {
        let list: Vec<String> = ctx.facts.iter().map(|f| format!("- {f}")).collect();
        parts.push(format!(
            "Saved memory facts (recorded earlier, may be stale):\n{}",
            list.join("\n")
        ));
    }
    let mut persona = Vec::new();
    if !ctx.style.is_empty() {
        persona.push(format!("Reply style: {}", ctx.style));
    }
    if !ctx.lang.is_empty() {
        persona.push(format!("Reply language: {}", ctx.lang));
    }
    if !ctx.signature.is_empty() {
        persona.push(format!("Signature: {}", ctx.signature));
    }
    if !persona.is_empty() {
        parts.push(persona.join("\n"));
    }
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

/// Consecutive messages of one role are joined and the reply starts with a user message, as provider APIs require.
#[must_use]
pub(crate) fn normalize(history: Vec<HistoryItem>) -> Vec<Turn> {
    let mut out: Vec<Turn> = Vec::new();
    for h in history {
        let role = if h.agent {
            TurnRole::Assistant
        } else {
            TurnRole::User
        };
        match out.last_mut() {
            Some(last) if last.role == role => {
                last.text.push_str("\n\n");
                last.text.push_str(&h.text);
            }
            None if role == TurnRole::Assistant => {}
            _ => out.push(Turn { role, text: h.text }),
        }
    }
    out
}

/// Account settings from the `models` page: `accounts.<id>.{base_url, model}`.
pub(crate) struct AccountSettings {
    pub base_url: Option<String>,
    pub model: Option<String>,
}

pub(crate) fn account_settings(doc: &Value, id: i64) -> AccountSettings {
    let a = &doc["accounts"][id.to_string()];
    let text = |k: &str| a[k].as_str().filter(|s| !s.is_empty()).map(str::to_string);
    AccountSettings {
        base_url: text("base_url"),
        model: text("model"),
    }
}

/// Context budget in tokens (`models.context_budget`), default 24000.
pub(crate) fn context_budget_tokens(doc: &Value) -> usize {
    doc["context_budget"]
        .as_u64()
        .filter(|n| *n > 0)
        .map_or(24_000, |n| usize::try_from(n).unwrap_or(24_000))
}

/// Model price: `models.prices.<model> = {input_per_m, output_per_m, currency?}`; no price means `None`.
pub(crate) fn price_of(doc: &Value, model: &str) -> Option<Price> {
    let p = &doc["prices"][model];
    Some(Price {
        input_per_m: p["input_per_m"].as_f64()?,
        output_per_m: p["output_per_m"].as_f64()?,
        currency: p["currency"].as_str().unwrap_or("USD").to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn h(agent: bool, t: &str) -> HistoryItem {
        HistoryItem {
            agent,
            text: t.into(),
        }
    }

    #[test]
    fn normalize_merges_and_starts_with_user() {
        let t = normalize(vec![
            h(true, "a0"),
            h(false, "u1"),
            h(false, "u2"),
            h(true, "a1"),
            h(false, "u3"),
        ]);
        let roles: Vec<_> = t.iter().map(|x| x.role).collect();
        assert_eq!(roles, [TurnRole::User, TurnRole::Assistant, TurnRole::User]);
        assert_eq!(t[0].text, "u1\n\nu2");
        assert!(normalize(vec![]).is_empty());
    }

    #[test]
    fn system_text_parts() {
        let mut c = ChatContext::default();
        assert_eq!(system_text(&c), None);
        c.agent_md = Some("  Rules\n".into());
        c.lang = "ru".into();
        c.signature = "— Me".into();
        assert_eq!(
            system_text(&c).unwrap(),
            "Rules\n\nReply language: ru\nSignature: — Me"
        );
    }

    #[test]
    fn models_page_readers() {
        let doc = json!({"accounts": {"3": {"base_url": "http://x/v1", "model": "m"}},
            "context_budget": 1000, "prices": {"m": {"input_per_m": 3.0, "output_per_m": 15.0}}});
        let a = account_settings(&doc, 3);
        assert_eq!(
            (a.base_url.as_deref(), a.model.as_deref()),
            (Some("http://x/v1"), Some("m"))
        );
        assert!(account_settings(&doc, 4).model.is_none());
        assert_eq!(context_budget_tokens(&doc), 1000);
        assert_eq!(context_budget_tokens(&json!({})), 24_000);
        let p = price_of(&doc, "m").unwrap();
        assert_eq!(p.currency, "USD");
        assert!(price_of(&doc, "other").is_none());
        assert!(price_of(&json!({"prices": {"m": {"input_per_m": 1.0}}}), "m").is_none());
    }
}
