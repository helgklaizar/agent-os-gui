//! `Secret<T>`: a value that never reaches `Debug`, `Display` and logs.

use std::fmt;

pub struct Secret<T>(T);

impl<T> Secret<T> {
    #[must_use]
    pub fn new(value: T) -> Secret<T> {
        Secret(value)
    }

    /// The only way to get the value out: the caller is responsible for where it goes.
    #[must_use]
    pub fn expose(&self) -> &T {
        &self.0
    }
}

impl<T> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

impl<T> fmt::Display for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

#[cfg(test)]
mod tests {
    use super::Secret;

    #[test]
    fn secret_never_prints_value() {
        let s = Secret::new("agt_super".to_string());
        assert_eq!(format!("{s:?}"), "<redacted>");
        assert_eq!(format!("{s}"), "<redacted>");
        assert_eq!(s.expose(), "agt_super");
    }
}
