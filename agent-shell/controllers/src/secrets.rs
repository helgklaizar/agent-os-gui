//! Masking secrets in displayed text (tokens, keys, passwords): permission cards do not show them.

const MASK: &str = "***";
const SECRET_KEYS: [&str; 9] = [
    "token",
    "access_token",
    "refresh_token",
    "secret",
    "password",
    "passwd",
    "api_key",
    "apikey",
    "authorization",
];
const SECRET_PREFIXES: [&str; 5] = ["agt_", "sk-", "ghp_", "glpat-", "xox"];

fn is_key(word: &str) -> bool {
    let k = word
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '_')
        .to_lowercase();
    SECRET_KEYS.contains(&k.as_str())
}

/// The value after the separator in a word like `key=value`, `"key":"value"`; `None` if the word is not of that form.
fn split_pair(word: &str) -> Option<(&str, &str)> {
    let at = word.find(['=', ':'])?;
    let (key, rest) = word.split_at(at);
    is_key(key).then_some((&word[..=at], &rest[1..]))
}

/// End of the value: closing quotes and punctuation stay in place.
fn mask_value(value: &str) -> String {
    let start = value.trim_start_matches(['"', '\'']).len();
    let lead = &value[..value.len() - start];
    let core = value[value.len() - start..].trim_end_matches(['"', '\'', ',', ';', ')', '}']);
    let tail = &value[value.len() - start + core.len()..];
    if core.is_empty() {
        value.to_string()
    } else {
        format!("{lead}{MASK}{tail}")
    }
}

/// Replaces token, key and password values with `***`; other text and whitespace are untouched.
pub fn mask_secrets(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut mask_next = false;
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String, mask_next: &mut bool| {
        if word.is_empty() {
            return;
        }
        let lower = word.to_lowercase();
        let masked = if *mask_next && !word.trim_matches(['"', '\'']).is_empty() {
            *mask_next = lower.trim_matches(['"', '\'']) == "bearer";
            if *mask_next {
                word.clone()
            } else {
                mask_value(word)
            }
        } else if let Some((head, value)) = split_pair(word) {
            *mask_next = value.trim_matches(['"', '\'']).is_empty();
            format!("{head}{}", mask_value(value))
        } else if SECRET_PREFIXES
            .iter()
            .any(|p| lower.trim_start_matches(['"', '\'']).starts_with(p))
        {
            mask_value(word)
        } else {
            *mask_next = is_key(word) || lower.trim_matches(['"', '\'']) == "bearer";
            word.clone()
        };
        out.push_str(&masked);
        word.clear();
    };
    for c in text.chars() {
        if c.is_whitespace() {
            flush(&mut word, &mut out, &mut mask_next);
            out.push(c);
        } else {
            word.push(c);
        }
    }
    flush(&mut word, &mut out, &mut mask_next);
    out
}
