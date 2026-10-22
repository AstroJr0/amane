use std::env;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

use serde_json::{json, Value};

use crate::Workspace;

fn socket() -> Option<PathBuf> {
    if let Some(path) = env::var_os("UMBRIEL_SOCKET").map(PathBuf::from) {
        if path.exists() {
            return Some(path);
        }
    }

    let runtime = env::var_os("XDG_RUNTIME_DIR")?;
    let display = env::var_os("WAYLAND_DISPLAY")
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_else(|| "wayland-0".into());

    let path = PathBuf::from(runtime).join(format!("umbriel-{display}.sock"));
    path.exists().then_some(path)
}

fn request(command: &str) -> Option<String> {
    let mut stream = UnixStream::connect(socket()?).ok()?;
    stream.write_all(format!("{command}\n").as_bytes()).ok()?;

    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply).ok()?;
    Some(reply)
}

pub fn focus_workspace(id: i64) {
    let Some(target) = target_of(id) else { return };
    let cmd = format!("workspace-switch:{target}");

    let req = json!({ "cmd": "msg", "arg": cmd });

    let success = request(&req.to_string()).is_some_and(|r| !r.contains(r#""err""#));
    if !success {
        let _ = Command::new("umbriel")
            .args(["msg", &cmd])
            .status();
    }
}


fn target_of(id: i64) -> Option<String> {
    if let Some(workspaces) = fetch() {
        if let Some(ws) = workspaces.iter().find(|w| w.id == id) {
            return ws.name.clone().or_else(|| Some(ws.index.to_string()));
        }
    }
    (id > 0).then(|| id.to_string())
}

pub fn listen(mut on_change: impl FnMut(Vec<Workspace>)) {
    if let Some(list) = fetch() {
        on_change(list);
    }

    let mut stream = match UnixStream::connect(socket()?) {
        Ok(s) => s,
        Err(_) => return,
    };

    let req = json!({ "cmd": "subscribe", "events": ["workspaces"] });
    if stream.write_all(format!("{req}\n").as_bytes()).is_err() {
        return;
    }

    let reader = BufReader::new(stream);
    for line in reader.lines().flatten() {
        if let Some(list) = parse(&line) {
            on_change(list);
        }
    }
}

fn fetch() -> Option<Vec<Workspace>> {
    // try socket
    if let Some(reply) = request(&json!({ "cmd": "workspaces" }).to_string()) {
        if let Some(list) = parse(&reply) {
            return Some(list);
        }
    }

    // if fails, extract json code of workspaces
    // by running `umbriel workspaces --json`
    let output = Command::new("umbriel").args(["workspaces", "--json"]).output().ok()?;
    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout);
        return parse(&text);
    }

    None
}


fn parse(line: &str) -> Option<Vec<Workspace>> {
    let value: Value = serde_json::from_str(line).ok()?;

    if let Some(event) = value.get("event").and_then(Value::as_str) {
        if event == "workspaces" || event == "workspace" {
            let data = value.get("data")
                .or_else(|| value.get("workspaces"))
                .and_then(Value::as_array)?;
            return parse_workspaces(data);
        }
        return None;
    }

    let array = value.get("ok")
        .or_else(|| value.get("workspaces"))
        .and_then(Value::as_array)
        .or_else(|| value.as_array())?;

    parse_workspaces(array).or_else(|| {
        // Prevent raw JSON fragments from falling through to the text parser
        let trimmed = line.trim_start();
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            None
        } else {
            parse_text_workspaces(line)
        }
    })
}

fn parse_workspaces(workspaces: &[Value]) -> Option<Vec<Workspace>> {
    let list: Vec<Workspace> = workspaces
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let index = w["index"]
                .as_u64()
                .or_else(|| w["index"].as_str().and_then(|s| s.parse().ok()))
                .or_else(|| w["idx"].as_u64())
                .or_else(|| w["idx"].as_str().and_then(|s| s.parse().ok()))
                .or_else(|| w["id"].as_u64())
                .or_else(|| w["id"].as_str().and_then(|s| s.parse().ok()))
                .unwrap_or((i + 1) as u64) as u32;

            let id = w["id"]
                .as_i64()
                .or_else(|| w["id"].as_str().and_then(|s| s.parse().ok()))
                .unwrap_or(index as i64);

            let display_name = w["name"]
                .as_str()
                .or_else(|| w["display_name"].as_str())
                .or_else(|| w["title"].as_str());

            let named = w["named"]
                .as_bool()
                .unwrap_or_else(|| display_name.is_some_and(|n| n != index.to_string()));

            let name = named.then(|| display_name.map(String::from))??;

            let output = w["output"]
                .as_str()
                .or_else(|| w["monitor"].as_str())
                .or_else(|| w["output_name"].as_str())
                .map(String::from);

            let focused = w["focused"]
                .as_bool()
                .or_else(|| w["is_focused"].as_bool())
                .unwrap_or(false);

            let active = w["active"]
                .as_bool()
                .or_else(|| w["is_active"].as_bool())
                .or_else(|| w["visible"].as_bool())
                .or_else(|| w["is_visible"].as_bool())
                .unwrap_or(focused);

            let urgent = w["urgent"]
                .as_bool()
                .or_else(|| w["is_urgent"].as_bool())
                .unwrap_or(false);

            let windows = w["window_count"]
                .as_u64()
                .or_else(|| w["windows"].as_u64())
                .or_else(|| w["windows"].as_array().map(|a| a.len() as u64))
                .or_else(|| w["window_count"].as_str().and_then(|s| s.parse().ok()))
                .unwrap_or(0) as u32;

            Workspace {
                id,
                index,
                name,
                output,
                active,
                focused,
                urgent,
                windows,
            }
        })
        .collect();

    (!list.is_empty()).then_some(list)
}

fn parse_text_workspaces(text: &str) -> Option<Vec<Workspace>> {
    let mut list = Vec::new();
    let mut current_id: i64 = 1;

    for part in text.split(|c| c == '\n' || c == '.') {
        let trimmed = part.trim().trim_start_matches('*').trim();
        if trimmed.is_empty() {
            continue;
        }

        let (out, rest) = trimmed.split_once(':')?;
        let output = out.trim();
        let rest = rest.trim();

        let focused = rest.contains("(focused)");
        let active = focused || rest.contains("(active)");

        let token = rest.split_whitespace().next()?;
        if token.is_empty() {
            continue;
        }

        let (index, name) = match token.parse::<u32>() {
            Ok(idx) => (idx, None),
            Err(_) => (current_id as u32, Some(token.to_string())),
        };

        list.push(Workspace {
            id: current_id,
            index,
            name,
            output: Some(output.to_string()),
            active,
            focused,
            urgent: false,
            windows: 0,
        });

        current_id += 1;
    }

    (!list.is_empty()).then_some(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORKSPACES_QUERY: &str = r#"{ "ok":[
            { "id":1, "index":1, "name":"1",   "named":false, "output":"eDP-1", "active":true,  "focused":true,  "urgent":false, "window_count":3 },
            { "id":2, "index":2, "name":"web", "named":true,  "output":"eDP-1", "active":false, "focused":false, "urgent":true,  "window_count":1 }
        ]}"#;

    #[test]
    fn reads_workspaces_from_query() {
        let list = parse(WORKSPACES_QUERY).expect("failed to parse query");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].windows, 3);
        assert_eq!(list[1].name.as_deref(), Some("web"));
    }

    #[test]
    fn follows_event_stream_snapshots_until_eof() {
        let first = r#"{"event":"workspaces","data":[{"id":1,"index":1,"name":"1","named":false,"output":"eDP-1","active":true,"focused":true,"urgent":false,"window_count":2}]}"#;
        let second = r#"{"event":"workspaces","data":[{"id":1,"index":1,"name":"1","named":false,"output":"eDP-1","active":true,"focused":true,"urgent":false,"window_count":5}]}"#;

        let input = format!("{first}\n{{\"event\":\"windows\",\"data\":[]}}\ninvalid\n{second}\n");

        let mut updates = Vec::new();
        let reader = std::io::Cursor::new(input);

        for line in reader.lines().flatten() {
            if let Some(list) = parse(&line) {
                updates.push(list);
            }
        }

        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0][0].windows, 2);
        assert_eq!(updates[1][0].windows, 5);
    }

    #[test]
    fn parses_plain_text_workspaces() {
        let text = "* DP-1: 1 [scrolling] (focused). DP-1: 2 [dwindle].\n* DP-2: WEB [master].";
        let list = parse(text).expect("failed to parse plain text workspaces");
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].output.as_deref(), Some("DP-1"));
        assert_eq!(list[0].index, 1);
        assert!(list[0].focused);
        assert!(list[0].active);

        assert_eq!(list[1].output.as_deref(), Some("DP-1"));
        assert_eq!(list[1].index, 2);
        assert!(!list[1].focused);

        assert_eq!(list[2].output.as_deref(), Some("DP-2"));
        assert_eq!(list[2].name.as_deref(), Some("WEB"));
    }

    #[test]
    fn parses_array_without_id_field() {
        let json = r#"[
            { "index": 1, "output": "DP-1", "focused": true },
            { "index": 2, "output": "DP-1", "focused": false }
        ]"#;
        let list = parse(json).expect("failed to parse array without id");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, 1);
        assert_eq!(list[0].index, 1);
        assert!(list[0].focused);
        assert!(list[0].active);
        assert_eq!(list[1].index, 2);
        assert!(!list[1].focused);
    }

    #[test]
    fn ignores_unrecognized_json_fragments() {
        assert!(parse(r#"{ "id":1, "index":1, "window_count":2 }"#).is_none());
        assert!(parse(r#"{"event":"windows","data":[]}"#).is_none());
        assert!(parse("]}").is_none());
    }
        }
             
