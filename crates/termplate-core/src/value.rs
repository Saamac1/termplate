//! Runtime values and variable types (spec §3).

use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Text(String),
    Number(f64),
    Bool(bool),
    List(Vec<String>),
}

impl Value {
    /// Text form used for display and for the command environment.
    /// Lists are newline separated (spec §3 "Lista").
    pub fn to_text(&self) -> String {
        match self {
            Value::Text(s) => s.clone(),
            Value::Number(n) => format_number(*n),
            Value::Bool(b) => b.to_string(),
            Value::List(items) => items.join("\n"),
        }
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            Value::Text(s) => s.trim().parse().ok(),
            Value::Bool(_) | Value::List(_) => None,
        }
    }

    /// Truthiness for conditions: `true`, non-zero numbers, non-empty text
    /// other than `false`/`0`, non-empty lists.
    pub fn truthy(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Number(n) => *n != 0.0,
            Value::Text(s) => !(s.is_empty() || s == "false" || s == "0"),
            Value::List(l) => !l.is_empty(),
        }
    }

    pub fn as_list(&self) -> Vec<String> {
        match self {
            Value::List(l) => l.clone(),
            other => other.to_text().lines().map(str::to_string).collect(),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_text())
    }
}

pub fn format_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 { format!("{}", n as i64) } else { format!("{n}") }
}

#[derive(Debug, Clone, PartialEq)]
pub enum VarType {
    Text,
    Number { range: Option<(f64, f64)> },
    Bool,
    List,
    Choice(Vec<String>),
}

impl VarType {
    /// Converts text (from a default, an input field or the coprocess
    /// protocol) into a value of this type.
    pub fn parse_value(&self, s: &str) -> Result<Value, String> {
        match self {
            VarType::Text => Ok(Value::Text(s.to_string())),
            VarType::Number { range } => {
                let n: f64 = s.trim().parse().map_err(|_| format!("`{s}` is not a number"))?;
                if let Some((lo, hi)) = range
                    && (n < *lo || n > *hi) {
                        return Err(format!("{} is outside {}..{}", format_number(n), format_number(*lo), format_number(*hi)));
                    }
                Ok(Value::Number(n))
            }
            VarType::Bool => match s {
                "true" | "on" | "yes" | "1" => Ok(Value::Bool(true)),
                "false" | "off" | "no" | "0" => Ok(Value::Bool(false)),
                _ => Err(format!("`{s}` is not a bool")),
            },
            VarType::List => Ok(Value::List(if s.is_empty() { vec![] } else { s.lines().map(str::to_string).collect() })),
            VarType::Choice(opts) => {
                if opts.iter().any(|o| o == s) {
                    Ok(Value::Text(s.to_string()))
                } else {
                    Err(format!("`{s}` is not one of {}", opts.join(", ")))
                }
            }
        }
    }

    pub fn zero(&self) -> Value {
        match self {
            VarType::Text => Value::Text(String::new()),
            VarType::Number { range } => Value::Number(range.map(|r| r.0).unwrap_or(0.0)),
            VarType::Bool => Value::Bool(false),
            VarType::List => Value::List(vec![]),
            VarType::Choice(o) => Value::Text(o.first().cloned().unwrap_or_default()),
        }
    }
}

/// Parses `A..B` (numbers).
pub fn parse_range(s: &str) -> Option<(f64, f64)> {
    let (a, b) = s.split_once("..")?;
    let lo: f64 = a.parse().ok()?;
    let hi: f64 = b.parse().ok()?;
    (lo <= hi).then_some((lo, hi))
}

/// Variable values by name.
pub type Vars = BTreeMap<String, Value>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_text_forms() {
        assert_eq!(Value::Number(3.0).to_text(), "3");
        assert_eq!(Value::Number(2.5).to_text(), "2.5");
        assert_eq!(Value::Bool(true).to_text(), "true");
        assert_eq!(Value::List(vec!["a".into(), "b".into()]).to_text(), "a\nb");
    }

    #[test]
    fn truthiness() {
        assert!(Value::Bool(true).truthy());
        assert!(!Value::Text("".into()).truthy());
        assert!(!Value::Text("false".into()).truthy());
        assert!(Value::Text("x".into()).truthy());
        assert!(!Value::Number(0.0).truthy());
        assert!(!Value::List(vec![]).truthy());
    }

    #[test]
    fn as_number_and_list() {
        assert_eq!(Value::Text(" 42 ".into()).as_number(), Some(42.0));
        assert_eq!(Value::Text("x".into()).as_number(), None);
        assert_eq!(Value::Text("a\nb".into()).as_list(), ["a", "b"]);
    }

    #[test]
    fn parse_typed_values() {
        let num = VarType::Number { range: Some((1.0, 9.0)) };
        assert_eq!(num.parse_value("3"), Ok(Value::Number(3.0)));
        assert!(num.parse_value("10").unwrap_err().contains("outside 1..9"));
        assert!(num.parse_value("x").is_err());
        assert_eq!(VarType::Bool.parse_value("false"), Ok(Value::Bool(false)));
        assert!(VarType::Bool.parse_value("maybe").is_err());
        let ch = VarType::Choice(vec!["fast".into(), "safe".into()]);
        assert_eq!(ch.parse_value("safe"), Ok(Value::Text("safe".into())));
        assert!(ch.parse_value("slow").is_err());
        assert_eq!(VarType::List.parse_value(""), Ok(Value::List(vec![])));
        assert_eq!(VarType::List.parse_value("a\nb"), Ok(Value::List(vec!["a".into(), "b".into()])));
    }

    #[test]
    fn zero_values() {
        assert_eq!(VarType::Number { range: Some((1.0, 9.0)) }.zero(), Value::Number(1.0));
        assert_eq!(VarType::Choice(vec!["a".into()]).zero(), Value::Text("a".into()));
    }

    #[test]
    fn ranges() {
        assert_eq!(parse_range("1..9"), Some((1.0, 9.0)));
        assert_eq!(parse_range("-5..5"), Some((-5.0, 5.0)));
        assert_eq!(parse_range("9..1"), None);
        assert_eq!(parse_range("1-9"), None);
    }
}
