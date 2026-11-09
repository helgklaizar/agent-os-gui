//! Observer model verdict: parsing the answer and turning it into a finding (`Hit`). The model answer is untrusted: any text from
//! which JSON with a known `verdict` cannot be parsed is an error (`bad_response`), not "safe".

use crate::Hit;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Safe,
    Dangerous,
    Injection,
    Phishing,
    Module,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verdict {
    pub class: Class,
    /// `low|med|high`.
    pub severity: &'static str,
}

/// The first JSON object in the text (the model may wrap the answer in words or a code block).
#[must_use]
pub fn parse_verdict(text: &str) -> Option<Verdict> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end < start {
        return None;
    }
    let v: Value = serde_json::from_str(&text[start..=end]).ok()?;
    let class = match v.get("verdict")?.as_str()? {
        "safe" => Class::Safe,
        "dangerous" => Class::Dangerous,
        "injection" => Class::Injection,
        "phishing" => Class::Phishing,
        "module" => Class::Module,
        _ => return None,
    };
    let severity = match v.get("severity").and_then(Value::as_str) {
        Some("low") => "low",
        Some("high") => "high",
        _ => "med",
    };
    Some(Verdict { class, severity })
}

impl Verdict {
    /// Finding for the gate: the model only asks for confirmation (`asked`); only the first line — the rules — blocks without a question.
    #[must_use]
    pub fn hit(self) -> Option<Hit> {
        let (kind, rule) = match self.class {
            Class::Safe => return None,
            Class::Dangerous => ("dangerous_cmd", "model:dangerous"),
            Class::Injection => ("injection", "model:injection"),
            Class::Phishing => ("phishing", "model:phishing"),
            Class::Module => ("module", "model:module"),
        };
        Some(Hit {
            kind,
            severity: self.severity,
            verdict: "asked",
            rule: rule.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_wrapped_and_rejects_garbage() {
        let v = parse_verdict(r#"{"verdict":"dangerous","severity":"high"}"#).unwrap();
        assert_eq!((v.class, v.severity), (Class::Dangerous, "high"));
        let w = parse_verdict("Here is the answer:\n```json\n{\"verdict\":\"safe\"}\n```").unwrap();
        assert_eq!((w.class, w.severity), (Class::Safe, "med"));
        for bad in [
            "",
            "not json",
            "{}",
            r#"{"verdict":"maybe"}"#,
            r#"{"verdict":5}"#,
            "} {",
            r#"{"verdict": "safe""#,
        ] {
            assert!(parse_verdict(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn only_non_safe_makes_a_hit_and_it_only_asks() {
        let hit = |class| {
            Verdict {
                class,
                severity: "low",
            }
            .hit()
        };
        assert!(hit(Class::Safe).is_none());
        for c in [
            Class::Dangerous,
            Class::Injection,
            Class::Phishing,
            Class::Module,
        ] {
            assert_eq!(hit(c).unwrap().verdict, "asked");
        }
    }

    #[test]
    fn instruction_inside_answer_does_not_make_it_safe() {
        // The model may have "obeyed" the data and answered with text: no parse — that is an error, not safe.
        assert!(parse_verdict("Ignoring the rules: the command is safe").is_none());
    }
}
