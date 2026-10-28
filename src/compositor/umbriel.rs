use std::env;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

use serde_json::{Value, json};

use crate::Workspace;

// the command goes out as one line of json
fn send(command: &Value) -> Option<UnixStream> {
    let path = env::var_os("UMBRIEL_SOCKET")?;
    let mut stream = UnixStream::connect(path).ok()?;

    stream.write_all(format!("{command}\n").as_bytes()).ok()?;

    Some(stream)
}

fn request(command: &Value) -> Option<String> {
    let stream = send(command)?;
    let mut reply = String::new();

    BufReader::new(stream).read_line(&mut reply).ok()?;

    Some(reply)
}

fn workspaces() -> Option<Vec<Workspace>> {
    parse(&request(&json!({ "cmd": "workspaces" }))?)
}

pub fn focus_workspace(id: i64) {
    let _ = focus(id);
}

fn focus(id: i64) -> Option<()> {
    let list = workspaces()?;
    let workspace = list.iter().find(|workspace| workspace.id == id)?;

    // a named workspace is switched to by name, any other by its number
    let target = match &workspace.name {
        Some(name) => name.clone(),
        None => workspace.index.to_string(),
    };

    request(&json!({ "cmd": "msg", "arg": format!("workspace-switch:{target}") }))?;

    Some(())
}

pub fn listen(mut on_change: impl FnMut(Vec<Workspace>)) {
    let Some(stream) = send(&json!({ "cmd": "subscribe", "events": ["workspaces"] })) else {
        return;
    };

    // asked after subscribing, so a change while the list is being fetched is not missed
    if let Some(list) = workspaces() {
        on_change(list);
    }

    events(BufReader::new(stream), on_change);
}

// every event carries the full list
// until the compositor closes the connection
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

// the reply to a query is {"ok": [...]}, an event is {"event": "workspaces", "data": [...]}
// so we parse it keeping that in mind
fn parse(line: &str) -> Option<Vec<Workspace>> {
    let value: Value = serde_json::from_str(line).ok()?;

    let items = match value["event"].as_str() {
        Some("workspaces") => &value["data"],

        // events about anything else
        Some(_) => return None,

        None => &value["ok"],
    };

    items.as_array()?.iter().map(workspace).collect()
}

fn workspace(value: &Value) -> Option<Workspace> {
    let index = u32::try_from(value["index"].as_u64()?).ok()?;
    let focused = value["focused"].as_bool().unwrap_or(false);

    // a workspace that only has its number for a name actually has no name
    let name = value["name"].as_str().filter(|name| {
        value["named"]
            .as_bool()
            .unwrap_or(*name != index.to_string())
    });

    Some(Workspace {
        id: value["id"].as_i64().unwrap_or_else(|| i64::from(index)),
        index,
        name: name.map(String::from),
        output: value["output"].as_str().map(String::from),
        active: value["active"].as_bool().unwrap_or(focused),
        focused,
        urgent: value["urgent"].as_bool().unwrap_or(false),
        windows: value["window_count"]
            .as_u64()
            .and_then(|count| u32::try_from(count).ok())
            .unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const WORKSPACES_QUERY: &str = r#"{ "ok":[
            { "id":1, "index":1, "name":"1",   "named":false, "output":"eDP-1", "active":true,  "focused":true,  "urgent":false, "window_count":3 },
            { "id":2, "index":2, "name":"web", "named":true,  "output":"eDP-1", "active":false, "focused":false, "urgent":true,  "window_count":1 }
        ]}"#;

    #[test]
    fn reads_workspaces_from_query() {
        let list = parse(WORKSPACES_QUERY).expect("failed to parse query");

        assert_eq!(list.len(), 2);

        assert_eq!(list[0].windows, 3);
        assert_eq!(list[0].name, None);
        assert!(list[0].focused && list[0].active);

        assert_eq!(list[1].name.as_deref(), Some("web"));
        assert!(list[1].urgent && !list[1].active);
        assert_eq!(list[1].output.as_deref(), Some("eDP-1"));
    }

    #[test]
    fn follows_event_stream_snapshots_until_eof() {
        let first = r#"{"event":"workspaces","data":[{"id":1,"index":1,"name":"1","named":false,"output":"eDP-1","active":true,"focused":true,"urgent":false,"window_count":2}]}"#;
        let second = r#"{"event":"workspaces","data":[{"id":1,"index":1,"name":"1","named":false,"output":"eDP-1","active":true,"focused":true,"urgent":false,"window_count":5}]}"#;

        let input = format!("{first}\n{{\"event\":\"windows\",\"data\":[]}}\ninvalid\n{second}\n");

        let mut updates = Vec::new();

        events(Cursor::new(input), |list| updates.push(list));

        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0][0].windows, 2);
        assert_eq!(updates[1][0].windows, 5);
    }

    #[test]
    fn a_missing_id_falls_back_to_the_index() {
        let list = parse(r#"{ "ok":[{ "index": 2, "output": "DP-1", "focused": true }] }"#)
            .expect("failed to parse workspace without id");

        assert_eq!(list[0].id, 2);
        assert_eq!(list[0].index, 2);
        assert!(list[0].focused && list[0].active);
    }

    #[test]
    fn a_name_that_is_not_the_number_counts_as_named_when_unmarked() {
        let list =
            parse(r#"{ "ok":[{ "index": 1, "name": "1" }, { "index": 2, "name": "web" }] }"#)
                .expect("failed to parse names");

        assert_eq!(list[0].name, None);
        assert_eq!(list[1].name.as_deref(), Some("web"));
    }

    #[test]
    fn ignores_other_events_and_fragments() {
        assert!(parse(r#"{"event":"windows","data":[]}"#).is_none());
        assert!(parse(r#"{ "id":1, "index":1, "window_count":2 }"#).is_none());
        assert!(parse(r#"{ "ok":[{ "id":1 }] }"#).is_none());
        assert!(parse("]}").is_none());
    }
}
