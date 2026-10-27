//! Shell-host expressions (spec §3).
//!
//! *Templates* (values, sources, `env` values) are text with `$NAME`,
//! `${NAME}` and `$(command)` substitutions; `"…"` groups and still
//! substitutes, `'…'` is literal, `\` escapes one character. A template that
//! is exactly one `$NAME` keeps the variable's type (lists stay lists).
//!
//! *Conditions*: `A == B`, `!=`, `<`, `>`, `<=`, `>=` (numeric when both
//! sides are numbers), `not`, `and`, `or`, `ok $(command)` and bare
//! templates (truthy). Operators must be separate words. No arithmetic.

use termplate_core::token::tokenize;

#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    Lit(String),
    Var(String),
    Cmd(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Template(pub Vec<Part>);

impl Template {
    /// True when evaluating it runs a command (`$( … )`).
    pub fn runs_commands(&self) -> bool {
        self.0.iter().any(|p| matches!(p, Part::Cmd(_)))
    }

    /// The variable name when the template is exactly one `$NAME`.
    pub fn single_var(&self) -> Option<&str> {
        match self.0.as_slice() {
            [Part::Var(v)] => Some(v),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Cond {
    Or(Vec<Cond>),
    And(Vec<Cond>),
    Not(Box<Cond>),
    Cmp(Template, Op, Template),
    Truthy(Template),
    Ok(String),
}

impl Cond {
    /// True when evaluating it runs a command (`ok $( … )`, `$( … )`).
    pub fn runs_commands(&self) -> bool {
        match self {
            Cond::Or(v) | Cond::And(v) => v.iter().any(Cond::runs_commands),
            Cond::Not(c) => c.runs_commands(),
            Cond::Cmp(a, _, b) => a.runs_commands() || b.runs_commands(),
            Cond::Truthy(t) => t.runs_commands(),
            Cond::Ok(_) => true,
        }
    }
}

fn is_name_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Finds the `)` matching a `$(` whose content starts at `chars[i]`.
fn matching_paren(chars: &[char], mut i: usize) -> Option<usize> {
    let mut depth = 1;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 1,
            '\'' => {
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    i += 1;
                }
            }
            '"' => {
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

pub fn parse_template(src: &str) -> Result<Template, String> {
    let chars: Vec<char> = src.trim().chars().collect();
    let mut parts = Vec::new();
    let mut lit = String::new();
    let mut in_dq = false;
    let mut i = 0;
    let flush = |lit: &mut String, parts: &mut Vec<Part>| {
        if !lit.is_empty() {
            parts.push(Part::Lit(std::mem::take(lit)));
        }
    };
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' => {
                match chars.get(i + 1) {
                    Some(n) => lit.push(*n),
                    None => lit.push('\\'),
                }
                i += 2;
                continue;
            }
            '"' => in_dq = !in_dq,
            '\'' if !in_dq => {
                let end = chars[i + 1..].iter().position(|&c| c == '\'').ok_or("unterminated ' quote")?;
                lit.extend(&chars[i + 1..i + 1 + end]);
                i += end + 2;
                continue;
            }
            '$' if chars.get(i + 1) == Some(&'(') => {
                let end = matching_paren(&chars, i + 2).ok_or("unterminated $( substitution")?;
                let cmd: String = chars[i + 2..end].iter().collect();
                if cmd.trim().is_empty() {
                    return Err("empty $( ) substitution".into());
                }
                flush(&mut lit, &mut parts);
                parts.push(Part::Cmd(cmd));
                i = end + 1;
                continue;
            }
            '$' if chars.get(i + 1) == Some(&'{') => {
                let end = chars[i + 2..].iter().position(|&c| c == '}').ok_or("unterminated ${")?;
                let name: String = chars[i + 2..i + 2 + end].iter().collect();
                if name.is_empty() || !name.starts_with(is_name_start) || !name.chars().all(is_name_char) {
                    return Err(format!("bad variable name `{name}`"));
                }
                flush(&mut lit, &mut parts);
                parts.push(Part::Var(name));
                i += end + 3;
                continue;
            }
            '$' if chars.get(i + 1).is_some_and(|c| is_name_start(*c)) => {
                let mut j = i + 1;
                while j < chars.len() && is_name_char(chars[j]) {
                    j += 1;
                }
                flush(&mut lit, &mut parts);
                parts.push(Part::Var(chars[i + 1..j].iter().collect()));
                i = j;
                continue;
            }
            c => lit.push(c),
        }
        i += 1;
    }
    if in_dq {
        return Err("unterminated \" quote".into());
    }
    flush(&mut lit, &mut parts);
    Ok(Template(parts))
}

pub fn parse_cond(src: &str) -> Result<Cond, String> {
    let tokens = tokenize(src).map_err(|e| e.0)?;
    if tokens.is_empty() {
        return Err("empty condition".into());
    }
    let words: Vec<(String, bool, &str)> =
        tokens.iter().map(|t| (t.text.clone(), t.quoted, &src[t.start..t.end])).collect();
    let mut p = CondParser { words, pos: 0 };
    let c = p.or()?;
    if p.pos < p.words.len() {
        return Err(format!("unexpected `{}` in condition", p.words[p.pos].0));
    }
    Ok(c)
}

struct CondParser<'a> {
    /// (text, quoted, raw source)
    words: Vec<(String, bool, &'a str)>,
    pos: usize,
}

impl CondParser<'_> {
    fn peek_kw(&self, kw: &str) -> bool {
        self.words.get(self.pos).is_some_and(|(t, q, _)| !q && t == kw)
    }

    fn or(&mut self) -> Result<Cond, String> {
        let mut items = vec![self.and()?];
        while self.peek_kw("or") {
            self.pos += 1;
            items.push(self.and()?);
        }
        Ok(if items.len() == 1 { items.pop().unwrap() } else { Cond::Or(items) })
    }

    fn and(&mut self) -> Result<Cond, String> {
        let mut items = vec![self.not()?];
        while self.peek_kw("and") {
            self.pos += 1;
            items.push(self.not()?);
        }
        Ok(if items.len() == 1 { items.pop().unwrap() } else { Cond::And(items) })
    }

    fn not(&mut self) -> Result<Cond, String> {
        if self.peek_kw("not") {
            self.pos += 1;
            return Ok(Cond::Not(Box::new(self.not()?)));
        }
        self.cmp()
    }

    fn operand(&mut self) -> Result<Template, String> {
        let Some((text, quoted, raw)) = self.words.get(self.pos).cloned() else {
            return Err("missing operand".into());
        };
        if !quoted && ["and", "or", "not", "==", "!=", "<", ">", "<=", ">="].contains(&text.as_str()) {
            return Err(format!("expected a value, found `{text}`"));
        }
        self.pos += 1;
        parse_template(raw)
    }

    fn cmp(&mut self) -> Result<Cond, String> {
        if self.peek_kw("ok") {
            self.pos += 1;
            let t = self.operand()?;
            return match t.0.as_slice() {
                [Part::Cmd(c)] => Ok(Cond::Ok(c.clone())),
                _ => Err("`ok` needs a $(command)".into()),
            };
        }
        let left = self.operand()?;
        let op = match self.words.get(self.pos) {
            Some((t, false, _)) => match t.as_str() {
                "==" => Some(Op::Eq),
                "!=" => Some(Op::Ne),
                "<" => Some(Op::Lt),
                ">" => Some(Op::Gt),
                "<=" => Some(Op::Le),
                ">=" => Some(Op::Ge),
                "=" => return Err("use `==` for comparison".into()),
                _ => None,
            },
            _ => None,
        };
        match op {
            Some(op) => {
                self.pos += 1;
                let right = self.operand()?;
                Ok(Cond::Cmp(left, op, right))
            }
            None => Ok(Cond::Truthy(left)),
        }
    }
}

/// Compares two texts: numerically when both are numbers.
pub fn compare(a: &str, op: Op, b: &str) -> bool {
    use std::cmp::Ordering;
    let ord = match (a.trim().parse::<f64>(), b.trim().parse::<f64>()) {
        (Ok(x), Ok(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
        _ => a.cmp(b),
    };
    match op {
        Op::Eq => ord == Ordering::Equal,
        Op::Ne => ord != Ordering::Equal,
        Op::Lt => ord == Ordering::Less,
        Op::Gt => ord == Ordering::Greater,
        Op::Le => ord != Ordering::Greater,
        Op::Ge => ord != Ordering::Less,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(s: &str) -> Part {
        Part::Lit(s.into())
    }
    fn var(s: &str) -> Part {
        Part::Var(s.into())
    }
    fn cmd(s: &str) -> Part {
        Part::Cmd(s.into())
    }

    #[test]
    fn template_variables() {
        assert_eq!(parse_template("$TARGET").unwrap().0, [var("TARGET")]);
        assert_eq!(parse_template("FILE: $it").unwrap().0, [lit("FILE: "), var("it")]);
        assert_eq!(parse_template("${A}x$B").unwrap().0, [var("A"), lit("x"), var("B")]);
        assert_eq!(parse_template("cost $5").unwrap().0, [lit("cost $5")]);
        assert_eq!(parse_template("$TARGET").unwrap().single_var(), Some("TARGET"));
        assert_eq!(parse_template("$A$B").unwrap().single_var(), None);
    }

    #[test]
    fn template_commands() {
        assert_eq!(parse_template("$(df -h / | tail -1)").unwrap().0, [cmd("df -h / | tail -1")]);
        assert_eq!(parse_template("$(echo $(date))").unwrap().0, [cmd("echo $(date)")]);
        assert_eq!(parse_template("$(echo ')')").unwrap().0, [cmd("echo ')'")]);
        assert_eq!(parse_template("$(echo \")\")").unwrap().0, [cmd("echo \")\"")]);
        assert_eq!(parse_template("$(cut -d' ' -f1 /proc/loadavg)").unwrap().0, [cmd("cut -d' ' -f1 /proc/loadavg")]);
    }

    #[test]
    fn template_quotes_and_escapes() {
        assert_eq!(parse_template("\"/mnt/my backup\"").unwrap().0, [lit("/mnt/my backup")]);
        assert_eq!(parse_template("\"a $X\"").unwrap().0, [lit("a "), var("X")]);
        assert_eq!(parse_template("'$X'").unwrap().0, [lit("$X")]);
        assert_eq!(parse_template("\\$X").unwrap().0, [lit("$X")]);
        assert_eq!(parse_template("\"\"").unwrap().0, []);
    }

    #[test]
    fn template_errors() {
        assert!(parse_template("$(oops").is_err());
        assert!(parse_template("$()").is_err());
        assert!(parse_template("\"open").is_err());
        assert!(parse_template("'open").is_err());
        assert!(parse_template("${}").is_err());
        assert!(parse_template("${A").is_err());
    }

    #[test]
    fn conditions_parse() {
        assert_eq!(
            parse_cond("$LEVEL > 5").unwrap(),
            Cond::Cmp(Template(vec![var("LEVEL")]), Op::Gt, Template(vec![lit("5")]))
        );
        assert_eq!(parse_cond("$TARGET == \"\"").unwrap(), Cond::Cmp(Template(vec![var("TARGET")]), Op::Eq, Template(vec![])));
        assert_eq!(parse_cond("not ok $(test -d $TARGET)").unwrap(), Cond::Not(Box::new(Cond::Ok("test -d $TARGET".into()))));
        assert_eq!(parse_cond("$VERBOSE").unwrap(), Cond::Truthy(Template(vec![var("VERBOSE")])));
    }

    #[test]
    fn precedence_not_and_or() {
        // a or b and not c  ==  a or (b and (not c))
        let c = parse_cond("$A or $B and not $C").unwrap();
        let Cond::Or(items) = c else { panic!() };
        assert_eq!(items.len(), 2);
        let Cond::And(inner) = &items[1] else { panic!() };
        assert!(matches!(inner[1], Cond::Not(_)));
    }

    #[test]
    fn condition_errors() {
        assert!(parse_cond("").is_err());
        assert!(parse_cond("$A ==").unwrap_err().contains("missing operand"));
        assert!(parse_cond("$A = 1").unwrap_err().contains("use `==`"));
        assert!(parse_cond("$A $B").unwrap_err().contains("unexpected"));
        assert!(parse_cond("ok $A").unwrap_err().contains("needs a $(command)"));
        assert!(parse_cond("and $A").is_err());
        assert!(parse_cond("not").is_err());
    }

    #[test]
    fn no_arithmetic() {
        // `+` is just text, not an operator
        assert!(parse_cond("$A + 1").is_err());
    }

    #[test]
    fn comparisons() {
        assert!(compare("10", Op::Gt, "9"), "numeric");
        assert!(compare("b", Op::Gt, "a"), "text");
        assert!(compare("3", Op::Eq, "3.0"));
        assert!(compare("a", Op::Ne, "b"));
        assert!(compare("1", Op::Le, "1") && compare("2", Op::Ge, "1"));
        assert!(compare("", Op::Eq, ""));
    }
}
