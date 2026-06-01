//! margo state + control via the `mctl` CLI. Queries use the socket
//! `get` topics (`mctl get clients|monitors|focused` → JSON); actions use
//! `mctl dispatch`. JSON→struct parsing is split into pure helpers so it
//! can be unit-tested without a running compositor.

use anyhow::{Context, Result};
use serde_json::Value;
use std::process::Command;

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
        monitor: o.get("monitor").and_then(Value::as_str).unwrap_or("").to_string(),
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
        active_tag_mask: o.get("active_tag_mask").and_then(Value::as_u64).unwrap_or(0) as u32,
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

// ── live mctl plumbing ─────────────────────────────────────

fn mctl_get(topic: &str) -> Result<Value> {
    let out = Command::new("mctl")
        .args(["get", topic])
        .output()
        .with_context(|| format!("running `mctl get {topic}`"))?;
    let v: Value =
        serde_json::from_slice(&out.stdout).context("parsing mctl JSON output")?;
    Ok(v)
}

/// `mctl dispatch <action> [args…]`.
pub fn dispatch(action: &str, args: &[&str]) -> Result<()> {
    let status = Command::new("mctl")
        .arg("dispatch")
        .arg(action)
        .args(args)
        .status()
        .with_context(|| format!("running `mctl dispatch {action}`"))?;
    anyhow::ensure!(status.success(), "mctl dispatch {action} failed");
    Ok(())
}

pub fn clients() -> Result<Value> {
    mctl_get("clients")
}
pub fn monitors() -> Result<Value> {
    mctl_get("monitors")
}
pub fn focused() -> Result<Value> {
    mctl_get("focused")
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
}
