//! Word splitting for `- ` item lines.
//!
//! Words are separated by whitespace. A double-quoted string is one word
//! (`\"` and `\\` are escapes). A `$( … )` substitution is kept whole even
//! when it contains spaces, so `value $(df -h / | tail -1) right` splits
//! into `value`, `$(df -h / | tail -1)` and `right`. Every token remembers
//! its byte span so expressions can be cut out of the original text verbatim.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// Token text. For a fully quoted token the quotes are removed and
    /// escapes resolved; otherwise the text is exactly as written.
    pub text: String,
    /// True when the whole token was a single double-quoted string.
    pub quoted: bool,
    pub start: usize,
    pub end: usize,
}

impl Token {
    pub fn is(&self, word: &str) -> bool {
        !self.quoted && self.text == word
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenizeError(pub String);

pub fn tokenize(s: &str) -> Result<Vec<Token>, TokenizeError> {
    let bytes: Vec<(usize, char)> = s.char_indices().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let (start, c) = bytes[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '"' {
            let (text, next) = read_quoted(s, &bytes, i)?;
            let end = if next < bytes.len() { bytes[next].0 } else { s.len() };
            // A quoted string directly followed by more text (`"a"b`) is one
            // unquoted word, like in the shell.
            if next < bytes.len() && !bytes[next].1.is_whitespace() {
                let (_, after) = read_bare(s, &bytes, next)?;
                let end = if after < bytes.len() { bytes[after].0 } else { s.len() };
                tokens.push(Token { text: s[start..end].to_string(), quoted: false, start, end });
                i = after;
            } else {
                tokens.push(Token { text, quoted: true, start, end });
                i = next;
            }
            continue;
        }
        let (_, next) = read_bare(s, &bytes, i)?;
        let end = if next < bytes.len() { bytes[next].0 } else { s.len() };
        tokens.push(Token { text: s[start..end].to_string(), quoted: false, start, end });
        i = next;
    }
    Ok(tokens)
}

/// Reads a quoted string starting at `bytes[i] == '"'`. Returns the unescaped
/// content and the index after the closing quote.
fn read_quoted(_s: &str, bytes: &[(usize, char)], i: usize) -> Result<(String, usize), TokenizeError> {
    let mut out = String::new();
    let mut j = i + 1;
    while j < bytes.len() {
        let c = bytes[j].1;
        match c {
            '\\' if j + 1 < bytes.len() && matches!(bytes[j + 1].1, '"' | '\\') => {
                out.push(bytes[j + 1].1);
                j += 2;
            }
            '"' => return Ok((out, j + 1)),
            _ => {
                out.push(c);
                j += 1;
            }
        }
    }
    Err(TokenizeError("unterminated string".into()))
}

/// Reads an unquoted word, keeping `$( … )` and embedded quotes intact.
/// Returns (unused, index after the word).
fn read_bare(s: &str, bytes: &[(usize, char)], mut j: usize) -> Result<((), usize), TokenizeError> {
    let mut depth = 0usize;
    while j < bytes.len() {
        let c = bytes[j].1;
        if depth == 0 && c.is_whitespace() {
            break;
        }
        if c == '"' {
            let (_, next) = read_quoted(s, bytes, j)?;
            j = next;
            continue;
        }
        if c == '\'' && depth > 0 {
            // single quotes inside $( … ) protect parentheses
            j += 1;
            while j < bytes.len() && bytes[j].1 != '\'' {
                j += 1;
            }
            if j >= bytes.len() {
                return Err(TokenizeError("unterminated ' quote".into()));
            }
            j += 1;
            continue;
        }
        if c == '$' && j + 1 < bytes.len() && bytes[j + 1].1 == '(' {
            depth += 1;
            j += 2;
            continue;
        }
        if c == '(' && depth > 0 {
            depth += 1;
        } else if c == ')' && depth > 0 {
            depth -= 1;
        }
        j += 1;
    }
    if depth > 0 {
        return Err(TokenizeError("unterminated $( substitution".into()));
    }
    Ok(((), j))
}

/// Joins the original source text of `tokens[from..to]`.
pub fn span_text<'a>(src: &'a str, tokens: &[Token], from: usize, to: usize) -> &'a str {
    if from >= to || from >= tokens.len() {
        return "";
    }
    &src[tokens[from].start..tokens[to - 1].end]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &str) -> Vec<String> {
        tokenize(s).unwrap().into_iter().map(|t| t.text).collect()
    }

    #[test]
    fn splits_on_whitespace() {
        assert_eq!(words("  TARGET  text   x "), ["TARGET", "text", "x"]);
    }

    #[test]
    fn quoted_string_is_one_token() {
        let t = tokenize(r#"TARGET text "/mnt/my backup""#).unwrap();
        assert_eq!(t[2].text, "/mnt/my backup");
        assert!(t[2].quoted);
        assert!(!t[0].quoted);
    }

    #[test]
    fn quoted_escapes() {
        let t = tokenize(r#"run "less -- \"$IT\"""#).unwrap();
        assert_eq!(t[1].text, r#"less -- "$IT""#);
    }

    #[test]
    fn empty_quoted_string() {
        let t = tokenize(r#"$TARGET == """#).unwrap();
        assert_eq!(t.len(), 3);
        assert_eq!(t[2].text, "");
        assert!(t[2].quoted);
    }

    #[test]
    fn command_substitution_is_one_token() {
        assert_eq!(
            words("d value $(df -h --output=avail / | tail -1) right"),
            ["d", "value", "$(df -h --output=avail / | tail -1)", "right"]
        );
    }

    #[test]
    fn nested_substitution_and_quotes() {
        assert_eq!(
            words(r#"x $(echo "a )" $(date +%s)) y"#),
            ["x", r#"$(echo "a )" $(date +%s))"#, "y"]
        );
        assert_eq!(words("x $(cut -d' ' -f1 /proc/loadavg)"), ["x", "$(cut -d' ' -f1 /proc/loadavg)"]);
    }

    #[test]
    fn embedded_quotes_stay_in_bare_word() {
        let t = tokenize(r#"env MSG="hello world" X=1"#).unwrap();
        assert_eq!(t[1].text, r#"MSG="hello world""#);
        assert!(!t[1].quoted);
        assert_eq!(t[2].text, "X=1");
    }

    #[test]
    fn spans_point_into_source() {
        let src = "value $(a b)   right";
        let t = tokenize(src).unwrap();
        assert_eq!(&src[t[1].start..t[1].end], "$(a b)");
        assert_eq!(span_text(src, &t, 0, 2), "value $(a b)");
        assert_eq!(span_text(src, &t, 1, 3), "$(a b)   right");
        assert_eq!(span_text(src, &t, 2, 2), "");
    }

    #[test]
    fn unterminated_errors() {
        assert!(tokenize(r#"a "b"#).is_err());
        assert!(tokenize("a $(b").is_err());
    }

    #[test]
    fn is_matches_only_unquoted() {
        let t = tokenize(r#"when "when""#).unwrap();
        assert!(t[0].is("when"));
        assert!(!t[1].is("when"));
    }
}
