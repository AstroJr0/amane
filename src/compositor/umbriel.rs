use std::env;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use serde_json::{json, Value};

use crate::Workspace;

// umbriel puts its socket path in UMBRIEL_SOCKET, or falls back to the runtime dir
fn socket_path() -> Option<PathBuf> {
    if let Some(path) = env::var_os("UMBRIEL_SOCKET") {
        return Some(PathBuf::from(path));
    }

    let runtime = env::var_os("XDG_RUNTIME_DIR")?;
    let display = env::var_os("WAYLAND_DISPLAY")
        .map(|display| display.to_string_lossy().into_owned())
        .unwrap_or_else(|| "wayland-0".to_string());

    Some(PathBuf::from(runtime).join(format!("umbriel-{display}.sock")))
}

fn connect() -> Option<UnixStream> {
    let path = socket_path()?;

    UnixStream::connect(path).ok()
}

// each request and reply is one json object per line
fn query(request: &str) -> Option<String> {
    let mut stream = connect()?;

    let line = format!("{request}\n");
    stream.write_all(line.as_bytes()).ok()?;

    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply).ok()?;

    Some(reply)
}

pub fn focus_workspace(id: i64) {
    let _ = focus(id, query);
}

fn focus(id: i64, mut query: impl FnMut(&str) -> Option<String>) -> Option<()> {
    let reply = query(r#"{"cmd":"workspaces"}"#)?;
    let list = parse(&reply)?;
    let target = focus_target(id, &list)?;

    let request = json!({
        "cmd": "msg",
        "arg": format!("workspace-switch:{target}")
    });

    let reply = query(&request.to_string())?;
    let value: Value = serde_json::from_str(&reply).ok()?;

    if value.get("err").is_some() {
        return None;
    }

    Some(())
}

fn focus_target(id: i64, list: &[Workspace]) -> Option<String> {
    let workspace = list.iter().find(|ws| ws.id == id)?;

    // bare digits select a position; quotes force a name so numeric names like "2" work
    let target = match &workspace.name {
        Some(name) => {
            if name.contains(['\n', '\r']) {
                return None;
            }

            format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""))
        }
        None => workspace.index.to_string(),
    };

    // umbriel scopes workspace actions to outputs with /output
    match &workspace.output {
        Some(output) => {
            if output.contains(['\n', '\r', '/', ':']) {
                return None;
            }

            Some(format!("{target}/{output}"))
        }
        None => Some(target),
    }
}

/*
 * umbriel emits full workspace snapshots on its event stream, so each
 * event replaces the entire list of workspaces
 */
pub fn listen(mut on_change: impl FnMut(Vec<Workspace>)) {
    let Some(mut stream) = connect() else {
        return;
    };

    let request = json!({
        "cmd": "subscribe",
        "events": ["workspaces"]
    });

    let line = format!("{request}\n");
    if stream.write_all(line.as_bytes()).is_err() {
        return;
    }

    events(BufReader::new(stream), on_change);
}

fn events(reader: impl BufRead, mut on_change: impl FnMut(Vec<Workspace>)) {
    for line in reader.lines() {
        let Ok(line) = line else {
            return;
        };

        if let Some(list) = parse(&line) {
            on_change(list);
        }
    }
}

fn parse(line: &str) -> Option<Vec<Workspace>> {
    let value: Value = serde_json::from_str(line).ok()?;

    // subscription events arrive as {"event":"workspaces","data":[...]}
    if let Some(event) = value.get("event").and_then(Value::as_str) {
        if event == "workspaces" {
            let data = value.get("data")?.as_array()?;
            return parse_workspaces(data);
        }

        return None;
    }

    // direct query replies arrive as {"ok":[...]}
    if let Some(ok) = value.get("ok").and_then(Value::as_array) {
        return parse_workspaces(ok);
    }

    if let Some(array) = value.as_array() {
        return parse_workspaces(array);
    }

    None
}

fn parse_workspaces(workspaces: &[Value]) -> Option<Vec<Workspace>> {
    let mut list = Vec::new();

    for workspace in workspaces {
        list.push(parse_workspace(workspace)?);
    }

    Some(list)
}

fn parse_workspace(value: &Value) -> Option<Workspace> {
    let id = value["id"].as_i64()?;
    let index = value["index"]
        .as_u64()
        .or_else(|| value["idx"].as_u64())? as u32;

    let display_name = value["name"]
        .as_str()
        .or_else(|| value["display_name"].as_str());

    // umbriel includes `named` so custom names can be distinguished from automatic numbers
    let named = value["named"]
        .as_bool()
        .unwrap_or_else(|| display_name.is_some_and(|name| name != index.to_string()));

    let name = if named {
        display_name.map(String::from)
    } else {
        None
    };

    let output = value["output"].as_str().map(String::from);

    let active = value["active"]
        .as_bool()
        .or_else(|| value["is_active"].as_bool())?;

    let focused = value["focused"]
        .as_bool()
        .or_else(|| value["is_focused"].as_bool())?;

    let urgent = value["urgent"]
        .as_bool()
        .or_else(|| value["is_urgent"].as_bool())
        .unwrap_or(false);

    let windows = value["window_count"]
        .as_u64()
        .or_else(|| value["windows"].as_u64())
        .unwrap_or(0) as u32;

    Some(Workspace {
        id,
        index,
        name,
        output,
        active,
        focused,
        urgent,
        windows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORKSPACES_QUERY: &str = r#"{"ok":[
        {"id":1,"index":1,"name":"1","named":false,"output":"eDP-1","active":true,"focused":true,"urgent":false,"window_count":3},
        {"id":2,"index":2,"name":"web","named":true,"output":"eDP-1","active":false,"focused":false,"urgent":true,"window_count":1},
        {"id":3,"index":1,"name":"3","named":true,"output":"DP-1","active":true,"focused":false,"urgent":false,"window_count":0}
    ]}"#;

    const WORKSPACE_EVENT: &str = r#"{"event":"workspaces","data":[{"id":1,"index":1,"name":"1","named":false,"output":"eDP-1","active":true,"focused":true,"urgent":false,"window_count":2}]}"#;

    #[test]
    fn follows_event_stream_snapshots_until_eof() {
        let first = WORKSPACE_EVENT.replace('\n', "");
        let updated = r#"{"event":"workspaces","data":[{"id":1,"index":1,"name":"1","named":false,"output":"eDP-1","active":true,"focused":true,"urgent":false,"window_count":5}]}"#;

        let input = format!("{first}\n{{\"event\":\"windows\",\"data\":[]}}\ninvalid\n{updated}\n");
        let mut updates = Vec::new();

        events(std::io::Cursor::new(input), |list| updates.push(list));

        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0][0].windows, 2);
        assert_eq!(updates[1][0].windows, 5);
    }

    #[test]
    fn focus_target_formats_proper_selectors() {
        let list = parse(WORKSPACES_QUERY).unwrap();

        // unnamed workspaces use their 1-based index scoped to their output
        assert_eq!(
            focus_target(list[0].id, &list),
            Some("1/eDP-1".to_string())
        );

        // named workspaces use quoted strings
        assert_eq!(
            focus_target(list[1].id, &list),
            Some("\"web\"/eDP-1".to_string())
        );

        // workspaces explicitly named with digits use quotes to avoid position matching
        assert_eq!(
            focus_target(list[2].id, &list),
            Some("\"3\"/DP-1".to_string())
        );

        assert!(focus_target(999, &list).is_none());
    }

    #[test]
    fn escapes_and_rejects_malformed_arguments() {
        let mut list = parse(WORKSPACES_QUERY).unwrap();

        // quotes and backslashes are escaped
        list[0].name = Some(r#"foo"bar\baz"#.to_string());
        assert_eq!(
            focus_target(list[0].id, &list),
            Some(r#""foo\"bar\\baz"/eDP-1"#.to_string())
        );

        // control characters and action delimiters are rejected
        for output in ["eDP-1/2", "eDP-1:2", "eDP-1\n", "eDP-1\r"] {
            list[0].output = Some(output.to_string());
            assert!(focus_target(list[0].id, &list).is_none());
        }

        list[0].output = Some("eDP-1".to_string());
        for name in ["foo\nbar", "foo\r"] {
            list[0].name = Some(name.to_string());
            assert!(focus_target(list[0].id, &list).is_none());
        }
    }

    #[test]
    fn focuses_workspace_with_switch_action() {
        let mut commands = Vec::new();

        let result = focus(2, |command| {
            commands.push(command.to_string());
            if command == r#"{"cmd":"workspaces"}"# {
                Some(WORKSPACES_QUERY.to_string())
            } else {
                Some(r#"{"ok":true}"#.to_string())
            }
        });

        assert_eq!(result, Some(()));
        assert_eq!(
            commands,
            [
                r#"{"cmd":"workspaces"}"#,
                r#"{"arg":"workspace-switch:\"web\"/eDP-1","cmd":"msg"}"#,
            ]
        );
    }

    #[test]
    fn follows_event_stream_snapshots_until_eof() {
        let updated = r#"{"event":"workspaces","data":[
            {"id":1,"index":1,"name":"1","named":false,"output":"eDP-1","active":true,"focused":true,"urgent":false,"window_count":5}
        ]}"#;

        let input = format!(
            "{WORKSPACE_EVENT}\n{{\"event\":\"windows\",\"data\":[]}}\ninvalid\n{updated}\n"
        );
        let mut updates = Vec::new();

        events(std::io::Cursor::new(input), |list| updates.push(list));

        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0][0].windows, 2);
        assert_eq!(updates[1][0].windows, 5);
    }

    #[test]
    fn skips_invalid_json_and_error_replies() {
        for line in [
            "not json",
            r#"{"err":"unsupported command"}"#,
            r#"{"event":"unknown","data":[]}"#,
            r#"{"ok":null}"#,
        ] {
            assert!(parse(line).is_none());
        }
    }
}
