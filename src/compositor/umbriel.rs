use std::env;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::Command;

use serde_json::{json, Value};

use crate::Workspace;

fn socket() -> Option<PathBuf> {
    if let Some(path) = env::var_os("UMBRIEL_SOCKET") {
        let p = PathBuf::from(path);
        if p.exists() {
            return Some(p);
        }
    }

    let runtime = env::var_os("XDG_RUNTIME_DIR")?;
    let display = env::var_os("WAYLAND_DISPLAY")
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_else(|| "wayland-0".to_string());

    let candidate = PathBuf::from(&runtime).join(format!("umbriel-{display}.sock"));
    if candidate.exists() {
        return Some(candidate);
    }

    // fallback: check any umbriel-*.sock in $XDG_RUNTIME_DIR
    if let Ok(entries) = std::fs::read_dir(&runtime) {
        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            if name.starts_with("umbriel-") && name.ends_with(".sock") {
                return Some(entry.path());
            }
        }
    }

    Some(candidate)
}

fn request(command: &str) -> Option<String> {
    let path = socket()?;
    let mut stream = UnixStream::connect(path).ok()?;

    let line = format!("{command}\n");
    stream.write_all(line.as_bytes()).ok()?;

    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply).ok()?;

    Some(reply)
}

pub fn focus_workspace(id: i64) {
    let Some(target) = target_of(id) else {
        return;
    };

    let req = json!({
        "cmd": "msg",
        "arg": format!("workspace-switch:{target}")
    });

    let ok = request(&req.to_string()).is_some_and(|r| !r.contains(r#""err""#));
    if !ok {
        // if fails, fallback to calling umbriel binary directly
        let _ = Command::new("umbriel")
            .args(["msg", &format!("workspace-switch:{target}")])
            .status();
    }
}

fn target_of(id: i64) -> Option<String> {
    if let Some(workspaces) = fetch() {
        if let Some(ws) = workspaces.iter().find(|w| w.id == id) {
            if let Some(ref name) = ws.name {
                return Some(name.clone());
            }
            return Some(ws.index.to_string());
        }
    }

    if id > 0 {
        return Some(id.to_string());
    }

    None
}

pub fn listen(mut on_change: impl FnMut(Vec<Workspace>)) {
    if let Some(list) = fetch() {
        on_change(list);
    }

    let Some(path) = socket() else {
        return;
    };

    let Ok(mut stream) = UnixStream::connect(path) else {
        return;
    };

    let req = json!({
        "cmd": "subscribe",
        "events": ["workspaces"]
    });

    let line = format!("{req}\n");
    if stream.write_all(line.as_bytes()).is_err() {
        return;
    }

    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let Ok(line) = line else {
            return;
        };

        if let Some(list) = parse(&line) {
            on_change(list);
        }
    }
}

fn fetch() -> Option<Vec<Workspace>> {
    let req = json!({ "cmd": "workspaces" });
    if let Some(reply) = request(&req.to_string()) {
        if let Some(list) = parse(&reply) {
            return Some(list);
        }
    }

    // fallback: `umbriel workspaces --json`
    // what it does: outputs result in a json format (similar to `hyprctl -j workspaces`)
    if let Ok(output) = Command::new("umbriel").args(["workspaces", "--json"]).output() {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            if let Some(list) = parse(&text) {
                return Some(list);
            }
        }
    }

    // fallback (2x) : `umbriel workspaces`
    // what it does: does the same thing, but outputs as a list, not as a json
    if let Ok(output) = Command::new("umbriel").arg("workspaces").output() {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            if let Some(list) = parse(&text) {
                return Some(list);
            }
        }
    }

    None
}

fn parse(line: &str) -> Option<Vec<Workspace>> {
    if let Ok(value) = serde_json::from_str::<Value>(line) {
        if let Some(event) = value.get("event").and_then(Value::as_str) {
            if event == "workspaces" || event == "workspace" {
                if let Some(data) = value.get("data").and_then(Value::as_array) {
                    return parse_workspaces(data);
                }
                if let Some(data) = value.get("workspaces").and_then(Value::as_array) {
                    return parse_workspaces(data);
                }
                // event notification without full list: it fetches the current snapshot
                return fetch();
            }
            return None;
        }

        if let Some(ok) = value.get("ok").and_then(Value::as_array) {
            return parse_workspaces(ok);
        }

        if let Some(workspaces) = value.get("workspaces").and_then(Value::as_array) {
            return parse_workspaces(workspaces);
        }

        if let Some(array) = value.as_array() {
            return parse_workspaces(array);
        }
    }

    // input that looks like json but matched no known shape (or it's a stray fragment)
    // so it must not fall through to the plain-text parser,
    // which would turn it into bogus workspaces.
    let trimmed = line.trim_start();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return None;
    }

    parse_text_workspaces(line)
}

fn parse_workspaces(workspaces: &[Value]) -> Option<Vec<Workspace>> {
    let mut list = Vec::new();

    for (i, workspace) in workspaces.iter().enumerate() {
        let index = workspace["index"]
            .as_u64()
            .or_else(|| workspace["index"].as_str().and_then(|s| s.parse().ok()))
            .or_else(|| workspace["idx"].as_u64())
            .or_else(|| workspace["idx"].as_str().and_then(|s| s.parse().ok()))
            .or_else(|| workspace["id"].as_u64())
            .or_else(|| workspace["id"].as_str().and_then(|s| s.parse().ok()))
            .unwrap_or((i + 1) as u64) as u32;

        let id = workspace["id"]
            .as_i64()
            .or_else(|| workspace["id"].as_str().and_then(|s| s.parse().ok()))
            .unwrap_or(index as i64);

        let display_name = workspace["name"]
            .as_str()
            .or_else(|| workspace["display_name"].as_str())
            .or_else(|| workspace["title"].as_str());

        let named = workspace["named"]
            .as_bool()
            .unwrap_or_else(|| display_name.is_some_and(|n| n != index.to_string()));

        let name = if named {
            display_name.map(String::from)
        } else {
            None
        };

        let output = workspace["output"]
            .as_str()
            .or_else(|| workspace["monitor"].as_str())
            .or_else(|| workspace["output_name"].as_str())
            .map(String::from);

        let focused = workspace["focused"]
            .as_bool()
            .or_else(|| workspace["is_focused"].as_bool())
            .unwrap_or(false);

        let active = workspace["active"]
            .as_bool()
            .or_else(|| workspace["is_active"].as_bool())
            .or_else(|| workspace["visible"].as_bool())
            .or_else(|| workspace["is_visible"].as_bool())
            .unwrap_or(focused);

        let urgent = workspace["urgent"]
            .as_bool()
            .or_else(|| workspace["is_urgent"].as_bool())
            .unwrap_or(false);

        let windows = workspace["window_count"]
            .as_u64()
            .or_else(|| workspace["windows"].as_u64())
            .or_else(|| workspace["windows"].as_array().map(|a| a.len() as u64))
            .or_else(|| workspace["window_count"].as_str().and_then(|s| s.parse().ok()))
            .unwrap_or(0) as u32;

        list.push(Workspace {
            id,
            index,
            name,
            output,
            active,
            focused,
            urgent,
            windows,
        });
    }

    if list.is_empty() {
        None
    } else {
        Some(list)
    }
}

// parses `umbriel workspaces` as a human-readable output, i.e.:
// * DP-1: 1 [scrolling] (focused). DP-1: 2 [dwindle].
// * DP-2: WEB [master].
fn parse_text_workspaces(text: &str) -> Option<Vec<Workspace>> {
    let mut list = Vec::new();
    let mut current_id: i64 = 1;

    for part in text.split(|c| c == '\n' || c == '.') {
        let trimmed = part.trim().trim_start_matches('*').trim();
        if trimmed.is_empty() {
            continue;
        }

        let Some((out, rest)) = trimmed.split_once(':') else {
            continue;
        };
        let output = out.trim();
        let rest = rest.trim();

        let focused = rest.contains("(focused)");
        let active = focused || rest.contains("(active)");

        let token = rest.split_whitespace().next().unwrap_or("");
        if token.is_empty() {
            continue;
        }

        let (index, name) = if let Ok(idx) = token.parse::<u32>() {
            (idx, None)
        } else {
            (current_id as u32, Some(token.to_string()))
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

    if list.is_empty() {
        None
    } else {
        Some(list)
    }
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
        // Stray JSON must not be turned into bogus workspaces by the text parser.
        assert!(parse(r#"{ "id":1, "index":1, "window_count":2 }"#).is_none());
        assert!(parse(r#"{"event":"windows","data":[]}"#).is_none());
        assert!(parse("]}").is_none());
    }
}
