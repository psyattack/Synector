//! Shared text helpers for catalog-driven labels.

/// Fills the `{}` placeholders of a catalog template left to right.
///
/// `str::replace` would fill every occurrence with the same value, so templates with more
/// than one placeholder are filled positionally instead.
pub fn fill(template: &str, args: &[&str]) -> String {
    let mut out = String::new();
    let mut rest = template;
    for arg in args {
        match rest.find("{}") {
            Some(at) => {
                out.push_str(&rest[..at]);
                out.push_str(arg);
                rest = &rest[at + 2..];
            }
            None => break,
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fill_replaces_each_placeholder_once() {
        assert_eq!(fill("{} {} {}", &["a", "b", "c"]), "a b c");
        assert_eq!(fill("{} {}", &["left", "right"]), "left right");
        assert_eq!(fill("{}", &[]), "{}");
        assert_eq!(fill("no placeholders", &["ignored"]), "no placeholders");
        // Positional, not all-occurrences: two placeholders get two different values.
        assert_eq!(fill("{}{}", &["b", "a"]), "ba");
    }
}
