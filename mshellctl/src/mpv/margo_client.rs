//! margo state + control for the mpv companion — via `mctl`'s own IPC
//! client library (`mctl::ipc_client`), talking to margo's Unix socket
//! directly rather than shelling out to the `mctl` binary (what `mplay`'s
//! equivalent module does). JSON→struct parsing is split into pure
//! helpers so it can be unit-tested without a running compositor.

use anyhow::{Result, bail};
use serde_json::Value;

/// A managed client (window) as margo reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Client {
    pub idx: i64,
    pub app_id: String,
    pub monitor: String,
    pub tags: u32,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub floating: bool,
}

/// An output (monitor) as margo reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub active: bool,
    pub active_tag_mask: u32,
}

fn obj_to_client(o: &Value) -> Option<Client> {
    Some(Client {
        idx: o.get("idx")?.as_i64()?,
        app_id: o.get("app_id")?.as_str()?.to_string(),
        monitor: o
            .get("monitor")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        tags: o.get("tags").and_then(Value::as_u64).unwrap_or(0) as u32,
        x: o.get("x").and_then(Value::as_i64).unwrap_or(0) as i32,
        y: o.get("y").and_then(Value::as_i64).unwrap_or(0) as i32,
        width: o.get("width").and_then(Value::as_i64).unwrap_or(0) as i32,
        height: o.get("height").and_then(Value::as_i64).unwrap_or(0) as i32,
        floating: o.get("floating").and_then(Value::as_bool).unwrap_or(false),
    })
}

fn obj_to_output(o: &Value) -> Option<Output> {
    Some(Output {
        name: o.get("name")?.as_str()?.to_string(),
        x: o.get("x").and_then(Value::as_i64).unwrap_or(0) as i32,
        y: o.get("y").and_then(Value::as_i64).unwrap_or(0) as i32,
        width: o.get("width").and_then(Value::as_i64).unwrap_or(0) as i32,
        height: o.get("height").and_then(Value::as_i64).unwrap_or(0) as i32,
        active: o.get("active").and_then(Value::as_bool).unwrap_or(false),
        active_tag_mask: o
            .get("active_tag_mask")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
    })
}

/// First client matching `app_id` in a `get clients` payload
/// (`{"clients":[…]}`).
pub fn find_client(v: &Value, app_id: &str) -> Option<Client> {
    v.get("clients")?
        .as_array()?
        .iter()
        .filter_map(obj_to_client)
        .find(|c| c.app_id == app_id)
}

/// Output named `name` in a `get monitors` payload (`{"monitors":[…]}`).
pub fn find_output(v: &Value, name: &str) -> Option<Output> {
    v.get("monitors")?
        .as_array()?
        .iter()
        .filter_map(obj_to_output)
        .find(|o| o.name == name)
}

/// The active output in a `get monitors` payload.
pub fn active_output(v: &Value) -> Option<Output> {
    v.get("monitors")?
        .as_array()?
        .iter()
        .filter_map(obj_to_output)
        .find(|o| o.active)
}

/// Parse a `get focused` payload (`{"focused":{…}}`) into a `Client`.
pub fn parse_focused(v: &Value) -> Option<Client> {
    obj_to_client(v.get("focused").unwrap_or(v))
}

// ── live margo IPC plumbing (via `mctl::ipc_client`, no subprocess) ─────────

fn margo_get(topic: &str) -> Result<Value> {
    mctl::ipc_client::request_once(&format!("get {topic}"))
        .map_err(|e| anyhow::anyhow!("querying margo for `{topic}`: {e}"))
}

/// Build the `dispatch <action> [args…]` wire line margo's socket parser
/// expects (a plain `split_whitespace()` grammar — see
/// `margo/src/ipc/protocol.rs`).
///
/// A bare `--` is silently dropped: it's only meaningful as a clap
/// flags-vs-positionals separator when going through the `mctl` CLI's own
/// argument parser (needed there so e.g. `-100` isn't mistaken for an
/// unknown flag) — this talks to the socket directly, so there's no clap
/// layer to strip it, and sending it verbatim becomes the action's first
/// (bogus) argument, silently corrupting it. Guards any caller that still
/// passes it out of `mctl`-CLI habit rather than requiring every call site
/// to remember not to.
fn build_dispatch_request(action: &str, args: &[&str]) -> String {
    let mut req = String::from("dispatch ");
    req.push_str(action);
    for a in args {
        if a.is_empty() || *a == "--" {
            continue;
        }
        req.push(' ');
        req.push_str(a);
    }
    req
}

/// `dispatch <action> [args…]` over margo's IPC socket.
pub fn dispatch(action: &str, args: &[&str]) -> Result<()> {
    let req = build_dispatch_request(action, args);
    let reply = mctl::ipc_client::request_once(&req)
        .map_err(|e| anyhow::anyhow!("dispatching `{action}`: {e}"))?;
    if reply.get("ok").and_then(Value::as_bool) != Some(true)
        && let Some(err) = reply.get("error").and_then(Value::as_str)
    {
        bail!("dispatch {action}: {err}");
    }
    Ok(())
}

pub fn clients() -> Result<Value> {
    margo_get("clients")
}
pub fn monitors() -> Result<Value> {
    margo_get("monitors")
}
pub fn focused() -> Result<Value> {
    margo_get("focused")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_clients_filters_app_id() {
        let j = json!({"clients":[
            {"idx":0,"app_id":"firefox","monitor":"DP-1","tags":1,"x":0,"y":0,"width":800,"height":600,"floating":false},
            {"idx":1,"app_id":"mpv","monitor":"DP-1","tags":4,"x":10,"y":20,"width":640,"height":360,"floating":true}
        ]});
        let c = find_client(&j, "mpv").unwrap();
        assert_eq!(c.idx, 1);
        assert_eq!(c.tags, 4);
        assert!(c.floating);
        assert_eq!((c.x, c.y, c.width, c.height), (10, 20, 640, 360));
        assert!(find_client(&j, "kitty").is_none());
    }

    #[test]
    fn parse_output_by_name() {
        let j = json!({"monitors":[
            {"name":"DP-1","x":0,"y":0,"width":1920,"height":1080,"active":true,"active_tag_mask":1}
        ]});
        let o = find_output(&j, "DP-1").unwrap();
        assert_eq!(o.width, 1920);
        assert!(o.active);
        assert!(find_output(&j, "HDMI-9").is_none());
    }

    #[test]
    fn active_output_picks_active_flag() {
        let j = json!({"monitors":[
            {"name":"A","x":0,"y":0,"width":1,"height":1,"active":false,"active_tag_mask":1},
            {"name":"B","x":0,"y":0,"width":2,"height":2,"active":true,"active_tag_mask":1}
        ]});
        assert_eq!(active_output(&j).unwrap().name, "B");
    }

    #[test]
    fn parse_focused_unwraps_wrapper() {
        let j = json!({"focused":{"idx":2,"app_id":"mpv","monitor":"DP-1","tags":8,
            "x":0,"y":0,"width":640,"height":360,"floating":true}});
        let c = parse_focused(&j).unwrap();
        assert_eq!(c.idx, 2);
        assert_eq!(c.app_id, "mpv");
    }

    #[test]
    fn dispatch_request_drops_bare_double_dash() {
        // Regression: a literal "--" (needed only when going through the
        // `mctl` CLI's clap parser) used to land in the wire request as
        // the action's first argument, since margo's socket grammar is a
        // plain whitespace split with no flag semantics — `movewin`/
        // `resizewin` silently received "--" as their x/y-offset instead
        // of the real value, and the mpv companion window never moved.
        assert_eq!(
            build_dispatch_request("movewin", &["--", "-100", "50"]),
            "dispatch movewin -100 50"
        );
        assert_eq!(build_dispatch_request("view", &["4"]), "dispatch view 4");
    }
}
