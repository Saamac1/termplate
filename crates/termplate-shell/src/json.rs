//! JSON Lines form of the coprocess protocol (`serve --json`, spec §14.3).
//!
//! Host → Termplate: `{"cmd":"set","name":"X","value":"…"}`, `{"cmd":"open","name":"M"}`,
//! `{"cmd":"close"}`, `{"cmd":"log","value":"…"}`, `{"cmd":"refresh"}`,
//! `{"cmd":"busy","value":true}`, `{"cmd":"quit"}`.
//! Termplate → host: `{"event":"select","name":"M","value":"ITEM"}`.

use serde_json::{json, Value as J};
use termplate_core::protocol::{Event, HostMsg};

pub fn encode_event(e: &Event) -> String {
    let mut obj = json!({ "event": e.kind, "name": e.name });
    if let Some(v) = &e.value {
        obj["value"] = J::String(v.clone());
    }
    obj.to_string()
}

pub fn parse_host_json(line: &str) -> Result<HostMsg, String> {
    let v: J = serde_json::from_str(line).map_err(|e| format!("bad JSON: {e}"))?;
    let s = |k: &str| v.get(k).and_then(J::as_str).map(str::to_string);
    let cmd = s("cmd").ok_or("missing \"cmd\"")?;
    let name = || s("name").ok_or_else(|| format!("`{cmd}` needs \"name\""));
    Ok(match cmd.as_str() {
        "set" => HostMsg::Set {
            name: name()?,
            value: match v.get("value") {
                Some(J::String(s)) => s.clone(),
                Some(J::Null) | None => String::new(),
                Some(J::Array(items)) => items.iter().map(|i| i.as_str().map(str::to_string).unwrap_or_else(|| i.to_string())).collect::<Vec<_>>().join("\n"),
                Some(other) => other.to_string(),
            },
        },
        "open" => HostMsg::Open(name()?),
        "close" => HostMsg::Close(s("name")),
        "log" => HostMsg::Log(match (s("name"), s("value")) {
            (Some(area), Some(text)) => format!("{area} {text}"),
            (None, Some(text)) => text,
            _ => return Err("`log` needs \"value\"".into()),
        }),
        "refresh" => HostMsg::Refresh,
        "busy" => HostMsg::Busy(v.get("value").and_then(J::as_bool).ok_or("`busy` needs a boolean \"value\"")?),
        "quit" => HostMsg::Quit,
        other => return Err(format!("unknown cmd `{other}`")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_encode_as_objects() {
        assert_eq!(
            encode_event(&Event::new("select", "MAIN_MENU", Some("BACKUP"))),
            r#"{"event":"select","name":"MAIN_MENU","value":"BACKUP"}"#
        );
        assert_eq!(encode_event(&Event::new("closed", "M", None)), r#"{"event":"closed","name":"M"}"#);
        assert_eq!(encode_event(&Event::new("input", "D", Some("a\n\"b\""))), r#"{"event":"input","name":"D","value":"a\n\"b\""}"#);
    }

    #[test]
    fn host_messages_parse() {
        assert_eq!(parse_host_json(r#"{"cmd":"set","name":"A","value":"x"}"#), Ok(HostMsg::Set { name: "A".into(), value: "x".into() }));
        assert_eq!(parse_host_json(r#"{"cmd":"set","name":"N","value":5}"#), Ok(HostMsg::Set { name: "N".into(), value: "5".into() }));
        assert_eq!(parse_host_json(r#"{"cmd":"set","name":"L","value":["a","b"]}"#), Ok(HostMsg::Set { name: "L".into(), value: "a\nb".into() }));
        assert_eq!(parse_host_json(r#"{"cmd":"open","name":"M"}"#), Ok(HostMsg::Open("M".into())));
        assert_eq!(parse_host_json(r#"{"cmd":"close"}"#), Ok(HostMsg::Close(None)));
        assert_eq!(parse_host_json(r#"{"cmd":"log","name":"LOG","value":"hi"}"#), Ok(HostMsg::Log("LOG hi".into())));
        assert_eq!(parse_host_json(r#"{"cmd":"busy","value":true}"#), Ok(HostMsg::Busy(true)));
        assert_eq!(parse_host_json(r#"{"cmd":"quit"}"#), Ok(HostMsg::Quit));
    }

    #[test]
    fn host_message_errors() {
        assert!(parse_host_json("not json").is_err());
        assert!(parse_host_json(r#"{"name":"x"}"#).is_err());
        assert!(parse_host_json(r#"{"cmd":"open"}"#).is_err());
        assert!(parse_host_json(r#"{"cmd":"busy","value":"on"}"#).is_err());
        assert!(parse_host_json(r#"{"cmd":"dance"}"#).is_err());
    }
}
