//! Coprocess protocol, line format (spec §14.3). The JSON Lines form lives
//! in the shell crate.
//!
//! A line is `VERB NAME VALUE…`; the value is the rest of the line with
//! newlines and backslashes escaped as `\n` and `\\`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostMsg {
    Set { name: String, value: String },
    Open(String),
    Close(Option<String>),
    /// `log [AREA] TEXT`; the engine decides whether the first word is an
    /// area name.
    Log(String),
    Refresh,
    Busy(bool),
    Quit,
}

/// Messages to the host script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub kind: String,
    pub name: String,
    pub value: Option<String>,
}

impl Event {
    pub fn new(kind: &str, name: &str, value: Option<&str>) -> Self {
        Event { kind: kind.into(), name: name.into(), value: value.map(str::to_string) }
    }
}

pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(o) => {
                out.push('\\');
                out.push(o);
            }
            None => out.push('\\'),
        }
    }
    out
}

pub fn encode_event(e: &Event) -> String {
    match &e.value {
        Some(v) => format!("{} {} {}", e.kind, e.name, escape(v)),
        None => format!("{} {}", e.kind, e.name),
    }
}

fn split_word(s: &str) -> (&str, &str) {
    let s = s.trim_start();
    match s.find(' ') {
        Some(i) => (&s[..i], &s[i + 1..]),
        None => (s, ""),
    }
}

pub fn parse_host_line(line: &str) -> Result<HostMsg, String> {
    let line = line.trim_end_matches(['\n', '\r']);
    let (verb, rest) = split_word(line);
    let msg = match verb {
        "set" => {
            let (name, value) = split_word(rest);
            if name.is_empty() {
                return Err("set needs a name".into());
            }
            HostMsg::Set { name: name.into(), value: unescape(value) }
        }
        "open" => match rest.trim() {
            "" => return Err("open needs a name".into()),
            n => HostMsg::Open(n.into()),
        },
        "close" => HostMsg::Close(Some(rest.trim()).filter(|s| !s.is_empty()).map(str::to_string)),
        "log" => HostMsg::Log(unescape(rest)),
        "refresh" => HostMsg::Refresh,
        "busy" => match rest.trim() {
            "on" => HostMsg::Busy(true),
            "off" => HostMsg::Busy(false),
            _ => return Err("busy takes on|off".into()),
        },
        "quit" => HostMsg::Quit,
        "" => return Err("empty message".into()),
        other => return Err(format!("unknown message `{other}`")),
    };
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping_roundtrip() {
        let s = "a\\b\nc\r";
        assert_eq!(escape(s), "a\\\\b\\nc\\r");
        assert_eq!(unescape(&escape(s)), s);
        assert_eq!(unescape("x\\ty\\"), "x\\ty\\");
    }

    #[test]
    fn encode_events() {
        assert_eq!(encode_event(&Event::new("select", "MAIN_MENU", Some("BACKUP"))), "select MAIN_MENU BACKUP");
        assert_eq!(encode_event(&Event::new("closed", "MAIN_MENU", None)), "closed MAIN_MENU");
        assert_eq!(encode_event(&Event::new("input", "DIR", Some("a\nb"))), "input DIR a\\nb");
    }

    #[test]
    fn parse_host_messages() {
        assert_eq!(
            parse_host_line("set STATUS_TEXT Valmistellaan nyt\n"),
            Ok(HostMsg::Set { name: "STATUS_TEXT".into(), value: "Valmistellaan nyt".into() })
        );
        assert_eq!(parse_host_line("set A"), Ok(HostMsg::Set { name: "A".into(), value: "".into() }));
        assert_eq!(parse_host_line("set A x\\ny"), Ok(HostMsg::Set { name: "A".into(), value: "x\ny".into() }));
        assert_eq!(parse_host_line("open MENU"), Ok(HostMsg::Open("MENU".into())));
        assert_eq!(parse_host_line("close"), Ok(HostMsg::Close(None)));
        assert_eq!(parse_host_line("close M"), Ok(HostMsg::Close(Some("M".into()))));
        assert_eq!(parse_host_line("log LOG done"), Ok(HostMsg::Log("LOG done".into())));
        assert_eq!(parse_host_line("refresh"), Ok(HostMsg::Refresh));
        assert_eq!(parse_host_line("busy on"), Ok(HostMsg::Busy(true)));
        assert_eq!(parse_host_line("quit"), Ok(HostMsg::Quit));
    }

    #[test]
    fn parse_errors() {
        assert!(parse_host_line("").is_err());
        assert!(parse_host_line("jump X").is_err());
        assert!(parse_host_line("busy maybe").is_err());
        assert!(parse_host_line("open").is_err());
        assert!(parse_host_line("set").is_err());
    }
}
