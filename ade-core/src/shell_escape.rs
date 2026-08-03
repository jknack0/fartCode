//! Shared shell-escaping utility (ticket E2-02, ARCHITECTURE.md §2 note).
//!
//! The codebase shells out to `git` via `std::process::Command` with argument
//! arrays, which never goes through a shell — so most call sites need **no**
//! quoting. This module exists for the cases that DO build shell command
//! strings (PTY spawn — E2-06, git command strings — E4), and as the single
//! canonical place for quoting rules ("no ad-hoc quoting anywhere").
//!
//! Rules:
//! - Prefer `Command` + args over a shell string; when a string is required,
//!   pass values through `single_quote` (POSIX) or `double_quote` (best
//!   effort on Windows cmd).
//! - Never interpolate a path/branch/ref into a shell string without quoting.

/// POSIX single-quote escaping: wrap in `'...'`, encode embedded `'` as the
/// canonical `'\''` sequence. Produces a string safe for `sh -c` / bash.
pub fn single_quote(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 2);
    out.push('\'');
    for c in input.chars() {
        if c == '\'' {
            // close quote, escaped quote, reopen quote
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// Windows-cmd double-quote escaping (best effort): wrap in `"..."`, escape
/// embedded `"` as `""` and `%` as `%%` (cmd variable-expansion). Only needed
/// on Windows; POSIX callers should use `single_quote`.
pub fn double_quote(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 2);
    out.push('"');
    for c in input.chars() {
        match c {
            '"' => out.push_str("\"\""),
            '%' => out.push_str("%%"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_quote_basic() {
        assert_eq!(single_quote("main"), "'main'");
        assert_eq!(single_quote("ade/fix-x"), "'ade/fix-x'");
    }

    #[test]
    fn single_quote_escapes_embedded_quotes() {
        assert_eq!(single_quote("it's"), "'it'\\''s'");
        // Round-trips through sh.
        let quoted = single_quote("a'b c");
        let output = std::process::Command::new("sh")
            .args(["-c", &format!("printf %s {quoted}")])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&output.stdout), "a'b c");
    }

    #[test]
    fn double_quote_escapes_cmd_specials() {
        assert_eq!(double_quote("a\"b"), "\"a\"\"b\"");
        assert_eq!(double_quote("100%"), "\"100%%\"");
    }
}
