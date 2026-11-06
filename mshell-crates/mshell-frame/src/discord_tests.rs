//! Tests for the Discord RPC session state machine (`Session`) and the
//! socket loop (`session`) against a fake Discord on a socket pair.
//!
//! Payload shapes follow Discord's RPC documentation
//! (discord.com/developers/docs/topics/rpc): responses echo the request
//! `nonce`, errors carry `evt: "ERROR"`, events come as
//! `cmd: "DISPATCH"` with `nonce: null`.

use super::*;
use std::os::unix::fs::PermissionsExt;

fn session_in(dir: &tempfile::TempDir) -> Session {
    Session::new(dir.path().join("margo/discord-token"))
}

/// The queued requests as (cmd, evt, args), clearing the queue.
fn sent(s: &mut Session) -> Vec<(String, Option<String>, Value)> {
    std::mem::take(&mut s.out)
        .into_iter()
        .map(|p| {
            (
                p["cmd"].as_str().unwrap_or_default().to_string(),
                p.get("evt").and_then(Value::as_str).map(str::to_string),
                p["args"].clone(),
            )
        })
        .collect()
}

/// Nonce of the last queued request with this `cmd` (without draining).
fn nonce_of(s: &Session, cmd: &str) -> String {
    s.out
        .iter()
        .rev()
        .find(|p| p["cmd"] == cmd)
        .and_then(|p| p["nonce"].as_str())
        .unwrap_or_else(|| panic!("no {cmd} queued"))
        .to_string()
}

fn reply(cmd: &str, nonce: &str, data: Value) -> Value {
    json!({ "cmd": cmd, "data": data, "evt": null, "nonce": nonce })
}

fn error(cmd: &str, nonce: &str, code: u32, message: &str) -> Value {
    json!({ "cmd": cmd, "data": { "code": code, "message": message }, "evt": "ERROR", "nonce": nonce })
}

fn dispatch(evt: &str, data: Value) -> Value {
    json!({ "cmd": "DISPATCH", "data": data, "evt": evt, "nonce": null })
}

fn authenticated_reply(nonce: &str) -> Value {
    reply(
        "AUTHENTICATE",
        nonce,
        json!({
            "application": { "id": STREAMKIT, "name": "StreamKit" },
            "expires": "2026-11-12T10:00:00.000000+00:00",
            "user": { "id": "10", "username": "kenan", "discriminator": "0" },
            "scopes": ["rpc", "rpc.voice.read", "rpc.voice.write"]
        }),
    )
}

const STREAMKIT: &str = discord::STREAMKIT_CLIENT_ID;

fn channel(id: &str, name: &str) -> Value {
    json!({
        "id": id, "name": name, "type": 2, "guild_id": "1",
        "voice_states": [
            { "nick": "kenan", "volume": 100,
              "voice_state": { "mute": false, "deaf": false, "self_mute": false, "self_deaf": false },
              "user": { "id": "10", "username": "kenan" } },
            { "volume": 80,
              "voice_state": { "mute": false, "deaf": false, "self_mute": true, "self_deaf": false },
              "user": { "id": "20", "username": "alice" } }
        ]
    })
}

/// A session that authenticated with `tok` and drained its queue.
fn ready_session(dir: &tempfile::TempDir) -> Session {
    let mut s = session_in(dir);
    s.authenticate("tok".into());
    let n = nonce_of(&s, "AUTHENTICATE");
    assert_eq!(s.on_payload(authenticated_reply(&n)), None);
    s.out.clear();
    s
}

/// Join `id` the way Discord reports it: select event, then the
/// GET_SELECTED_VOICE_CHANNEL answer.
fn join(s: &mut Session, id: &str, name: &str) -> Vec<(String, Option<String>, Value)> {
    s.on_payload(dispatch(
        "VOICE_CHANNEL_SELECT",
        json!({ "channel_id": id, "guild_id": "1" }),
    ));
    let n = nonce_of(s, "GET_SELECTED_VOICE_CHANNEL");
    s.out.clear();
    s.on_payload(reply("GET_SELECTED_VOICE_CHANNEL", &n, channel(id, name)));
    sent(s)
}

#[test]
fn stored_token_authenticates_saves_and_subscribes() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session_in(&dir);
    assert_eq!(s.state.status, DiscordStatus::NeedsAuth);
    s.authenticate("tok".into());
    assert_eq!(s.state.status, DiscordStatus::Authorizing);
    let n = nonce_of(&s, "AUTHENTICATE");
    let out = sent(&mut s);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].2, json!({ "access_token": "tok" }));

    assert_eq!(s.on_payload(authenticated_reply(&n)), None);
    assert_eq!(s.state.status, DiscordStatus::Ready);
    assert_eq!(s.state.user_id, "10");
    assert!(s.state.message.is_empty());
    assert_eq!(load_token(&s.token_file).as_deref(), Some("tok"));
    let mode = std::fs::metadata(&s.token_file)
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600, "token file is private");

    let out = sent(&mut s);
    let cmds: Vec<(&str, Option<&str>)> = out
        .iter()
        .map(|(c, e, _)| (c.as_str(), e.as_deref()))
        .collect();
    assert_eq!(
        cmds,
        [
            ("SUBSCRIBE", Some("VOICE_CHANNEL_SELECT")),
            ("SUBSCRIBE", Some("VOICE_SETTINGS_UPDATE")),
            ("SUBSCRIBE", Some("VOICE_CONNECTION_STATUS")),
            ("GET_SELECTED_VOICE_CHANNEL", None),
            ("GET_VOICE_SETTINGS", None),
        ]
    );
}

#[test]
fn rejected_token_is_deleted_and_asks_for_auth() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session_in(&dir);
    save_token(&s.token_file, "old");
    s.authenticate(load_token(&s.token_file).unwrap());
    let n = nonce_of(&s, "AUTHENTICATE");
    s.out.clear();
    s.on_payload(error("AUTHENTICATE", &n, 4009, "Invalid access token"));
    assert_eq!(s.state.status, DiscordStatus::NeedsAuth);
    assert_eq!(s.state.message, "Authorization expired — authorize again");
    assert_eq!(s.token, None);
    assert!(!s.token_file.exists(), "stale token removed");
    assert!(s.out.is_empty(), "nothing subscribed without auth");
}

#[test]
fn authorize_flow_returns_code_then_authenticates_with_exchanged_token() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session_in(&dir);
    s.on_command(DiscordCommand::Authorize);
    assert_eq!(s.state.status, DiscordStatus::Authorizing);
    let n = nonce_of(&s, "AUTHORIZE");
    let out = sent(&mut s);
    assert_eq!(out[0].2["client_id"], STREAMKIT);
    assert_eq!(
        out[0].2["scopes"],
        json!(["rpc", "rpc.voice.read", "rpc.voice.write"])
    );

    // A second click while the consent prompt is open sends nothing.
    s.on_command(DiscordCommand::Authorize);
    assert!(s.out.is_empty());

    let code = s.on_payload(reply("AUTHORIZE", &n, json!({ "code": "abc123" })));
    assert_eq!(code.as_deref(), Some("abc123"));
    assert_eq!(s.state.status, DiscordStatus::Authorizing);

    s.on_token_exchange(Ok("fresh".into()));
    let n = nonce_of(&s, "AUTHENTICATE");
    assert_eq!(sent(&mut s)[0].2, json!({ "access_token": "fresh" }));
    s.on_payload(authenticated_reply(&n));
    assert_eq!(s.state.status, DiscordStatus::Ready);
    assert_eq!(load_token(&s.token_file).as_deref(), Some("fresh"));
}

#[test]
fn authorize_denied_or_incomplete_or_exchange_failed_returns_to_needs_auth() {
    let dir = tempfile::tempdir().unwrap();

    // User pressed Cancel in Discord's consent prompt.
    let mut s = session_in(&dir);
    s.on_command(DiscordCommand::Authorize);
    let n = nonce_of(&s, "AUTHORIZE");
    let denied = "OAuth2 Error: access_denied: The resource owner or authorization server denied the request";
    assert_eq!(s.on_payload(error("AUTHORIZE", &n, 5000, denied)), None);
    assert_eq!(s.state.status, DiscordStatus::NeedsAuth);
    assert_eq!(s.state.message, denied);

    // Answer without a code.
    s.on_command(DiscordCommand::Authorize);
    assert!(s.state.message.is_empty(), "a retry clears the old error");
    let n = nonce_of(&s, "AUTHORIZE");
    assert_eq!(s.on_payload(reply("AUTHORIZE", &n, json!({}))), None);
    assert_eq!(s.state.status, DiscordStatus::NeedsAuth);
    assert_eq!(s.state.message, "Authorization was not completed");

    // The StreamKit broker failed.
    s.on_command(DiscordCommand::Authorize);
    s.on_token_exchange(Err("token exchange failed: timeout".into()));
    assert_eq!(s.state.status, DiscordStatus::NeedsAuth);
    assert_eq!(s.state.message, "token exchange failed: timeout");
    assert!(!s.token_file.exists());
}

#[test]
fn unknown_nonce_and_subscribe_acks_change_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ready_session(&dir);
    let before = s.state.clone();
    s.on_payload(reply("GET_VOICE_SETTINGS", "999", json!({ "mute": true })));
    s.on_payload(reply(
        "SUBSCRIBE",
        "1",
        json!({ "evt": "VOICE_CHANNEL_SELECT" }),
    ));
    assert_eq!(s.state, before);
    assert!(s.out.is_empty());
}

#[test]
fn a_nonce_is_answered_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ready_session(&dir);
    s.request("GET_VOICE_SETTINGS", json!({}), Kind::VoiceSettings, None);
    let n = nonce_of(&s, "GET_VOICE_SETTINGS");
    s.on_payload(reply("GET_VOICE_SETTINGS", &n, json!({ "mute": true })));
    assert!(s.state.self_mute);
    assert!(!s.pending.contains_key(&n));
    // A replayed answer is no longer matched to the request.
    s.on_payload(reply("GET_VOICE_SETTINGS", &n, json!({ "mute": false })));
    assert!(s.state.self_mute);
}

#[test]
fn nonces_are_unique() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session_in(&dir);
    for _ in 0..3 {
        s.request("GET_VOICE_SETTINGS", json!({}), Kind::Other, None);
    }
    let nonces: std::collections::HashSet<_> = s
        .out
        .iter()
        .map(|p| p["nonce"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(nonces.len(), 3);
    assert_eq!(s.pending.len(), 3);
}

#[test]
fn joining_a_channel_subscribes_its_events() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ready_session(&dir);
    let out = join(&mut s, "100", "General");
    assert!(s.state.in_voice());
    assert_eq!(s.state.channel_name, "General");
    assert_eq!(s.state.participants.len(), 2);
    assert_eq!(s.subscribed.as_deref(), Some("100"));
    // The channel's events, then its server's name (once per join).
    assert_eq!(out.len(), CHANNEL_EVENTS.len() + 1);
    assert_eq!(out[CHANNEL_EVENTS.len()].0, "GET_GUILD");
    assert_eq!(out[CHANNEL_EVENTS.len()].2, json!({ "guild_id": "1" }));
    for ((cmd, evt, args), want) in out.iter().zip(CHANNEL_EVENTS) {
        assert_eq!(cmd, "SUBSCRIBE");
        assert_eq!(evt.as_deref(), Some(want));
        assert_eq!(args, &json!({ "channel_id": "100" }));
    }

    // Refreshing the same channel does not re-subscribe.
    s.request(
        "GET_SELECTED_VOICE_CHANNEL",
        json!({}),
        Kind::SelectedChannel,
        None,
    );
    let n = nonce_of(&s, "GET_SELECTED_VOICE_CHANNEL");
    s.out.clear();
    s.on_payload(reply(
        "GET_SELECTED_VOICE_CHANNEL",
        &n,
        channel("100", "General"),
    ));
    assert!(s.out.is_empty());
}

#[test]
fn switching_channels_moves_the_subscriptions() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ready_session(&dir);
    join(&mut s, "100", "General");
    let out = join(&mut s, "200", "Gaming");
    assert_eq!(s.state.channel_name, "Gaming");
    assert_eq!(s.subscribed.as_deref(), Some("200"));
    let (unsub, sub): (Vec<_>, Vec<_>) = out
        .iter()
        .filter(|(c, _, _)| c != "GET_GUILD")
        .partition(|(c, _, _)| c == "UNSUBSCRIBE");
    assert_eq!(unsub.len(), CHANNEL_EVENTS.len());
    assert_eq!(sub.len(), CHANNEL_EVENTS.len());
    assert!(unsub.iter().all(|(_, _, a)| a["channel_id"] == "100"));
    assert!(
        sub.iter()
            .all(|(c, _, a)| c == "SUBSCRIBE" && a["channel_id"] == "200")
    );
    // Old subscriptions are dropped before the new ones are made.
    assert_eq!(out[0].0, "UNSUBSCRIBE");
}

#[test]
fn leaving_in_discord_unsubscribes() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ready_session(&dir);
    join(&mut s, "100", "General");
    s.on_payload(dispatch(
        "VOICE_CHANNEL_SELECT",
        json!({ "channel_id": null, "guild_id": null }),
    ));
    assert!(!s.state.in_voice());
    assert_eq!(s.subscribed, None);
    let out = sent(&mut s);
    assert_eq!(out.len(), CHANNEL_EVENTS.len());
    assert!(
        out.iter()
            .all(|(c, _, a)| c == "UNSUBSCRIBE" && a["channel_id"] == "100")
    );
}

#[test]
fn leave_command_disconnects_and_unsubscribes() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ready_session(&dir);
    join(&mut s, "100", "General");
    s.on_command(DiscordCommand::Leave);
    let out = sent(&mut s);
    assert_eq!(out[0].0, "SELECT_VOICE_CHANNEL");
    assert_eq!(out[0].2, json!({ "channel_id": null }));
    assert_eq!(out.len(), 1 + CHANNEL_EVENTS.len());
    assert!(out[1..].iter().all(|(c, _, _)| c == "UNSUBSCRIBE"));
    assert!(!s.state.in_voice());
    // Discord's echo of the leave changes nothing more.
    s.on_payload(dispatch(
        "VOICE_CHANNEL_SELECT",
        json!({ "channel_id": null }),
    ));
    assert!(s.out.is_empty());
}

#[test]
fn not_in_voice_answer_is_idle() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ready_session(&dir);
    s.request(
        "GET_SELECTED_VOICE_CHANNEL",
        json!({}),
        Kind::SelectedChannel,
        None,
    );
    let n = nonce_of(&s, "GET_SELECTED_VOICE_CHANNEL");
    s.out.clear();
    s.on_payload(reply("GET_SELECTED_VOICE_CHANNEL", &n, Value::Null));
    assert!(!s.state.in_voice());
    assert!(s.out.is_empty(), "nothing to subscribe");
}

#[test]
fn voice_events_update_participants() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ready_session(&dir);
    join(&mut s, "100", "General");
    s.on_payload(dispatch(
        "SPEAKING_START",
        json!({ "channel_id": "100", "user_id": "20" }),
    ));
    assert!(s.state.participants["20"].speaking);
    s.on_payload(dispatch(
        "VOICE_STATE_UPDATE",
        json!({ "voice_state": { "mute": false, "deaf": false, "self_mute": false, "self_deaf": true },
                "user": { "id": "20", "username": "alice" }, "volume": 80 }),
    ));
    let alice = &s.state.participants["20"];
    assert!(alice.deafened && !alice.muted && alice.speaking);
    s.on_payload(dispatch(
        "VOICE_SETTINGS_UPDATE",
        json!({ "mute": true, "deaf": false }),
    ));
    assert!(s.state.self_mute);
    s.on_payload(dispatch(
        "VOICE_STATE_DELETE",
        json!({ "user": { "id": "20" } }),
    ));
    assert!(!s.state.participants.contains_key("20"));
    assert!(s.out.is_empty(), "events never trigger requests here");
}

#[test]
fn errors_on_other_requests_only_show_a_message() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ready_session(&dir);
    s.on_command(DiscordCommand::SetMute(true));
    let n = nonce_of(&s, "SET_VOICE_SETTINGS");
    s.out.clear();
    s.on_payload(error("SET_VOICE_SETTINGS", &n, 4000, "Invalid payload"));
    assert_eq!(s.state.status, DiscordStatus::Ready);
    assert_eq!(s.state.message, "Invalid payload");
    assert!(s.token_file.exists(), "token kept");
    // An error without a message still says something.
    s.on_payload(json!({ "cmd": "SUBSCRIBE", "evt": "ERROR", "data": {}, "nonce": "77" }));
    assert_eq!(s.state.message, "Discord RPC error");
}

#[test]
fn voice_commands_need_ready_and_clamp_volume() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session_in(&dir);
    for c in [
        DiscordCommand::SetMute(true),
        DiscordCommand::SetDeaf(true),
        DiscordCommand::Leave,
        DiscordCommand::SetUserVolume("20".into(), 50),
    ] {
        s.on_command(c);
    }
    assert!(s.out.is_empty(), "nothing sent before authentication");

    let mut s = ready_session(&dir);
    s.on_command(DiscordCommand::Authorize);
    assert!(s.out.is_empty(), "authorize is only for NeedsAuth");
    s.on_command(DiscordCommand::SetMute(true));
    s.on_command(DiscordCommand::SetDeaf(false));
    let out = sent(&mut s);
    assert_eq!(out[0].2, json!({ "mute": true }));
    assert_eq!(out[1].2, json!({ "deaf": false }));

    join(&mut s, "100", "General");
    s.on_command(DiscordCommand::SetUserVolume("20".into(), 350));
    assert_eq!(
        s.state.participants["20"].volume, 200,
        "optimistic, clamped"
    );
    let out = sent(&mut s);
    assert_eq!(out[0].0, "SET_USER_VOICE_SETTINGS");
    assert_eq!(out[0].2, json!({ "user_id": "20", "volume": 200 }));
}

#[test]
fn token_storage_trims_and_ignores_empty() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("margo/discord-token");
    assert_eq!(load_token(&path), None);
    save_token(&path, "abc");
    assert_eq!(load_token(&path).as_deref(), Some("abc"));
    std::fs::write(&path, "  abc\n").unwrap();
    assert_eq!(load_token(&path).as_deref(), Some("abc"));
    std::fs::write(&path, "\n").unwrap();
    assert_eq!(load_token(&path), None);
    // Saving a shorter token truncates the old one.
    save_token(&path, "longer-token");
    save_token(&path, "x");
    assert_eq!(load_token(&path).as_deref(), Some("x"));
    clear_token(&path);
    assert!(!path.exists());
}

// ── The socket loop against a fake Discord ─────────────────────────────────

#[test]
fn a_join_is_remembered_with_its_server_name() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ready_session(&dir);
    let out = join(&mut s, "300", "General");
    let n = nonce_of_sent(&out, &s, "GET_GUILD");
    assert_eq!(s.state.history.recent.len(), 1);
    assert_eq!(s.state.history.recent[0].id, "300");
    assert_eq!(s.state.history.recent[0].guild_id, "1");
    s.on_payload(reply(
        "GET_GUILD",
        &n,
        json!({ "id": "1", "name": "Margo", "icon_url": null, "members": [] }),
    ));
    assert_eq!(s.state.guild_name, "Margo");
    // Persisted beside the token, private, and read back by a new session.
    let file = dir.path().join("margo/discord-channels.json");
    let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let again = session_in(&dir);
    assert_eq!(again.state.history.recent[0].guild_name, "Margo");
    // A refresh of the same channel is not a new join.
    let before = s.state.history.clone();
    let n = {
        s.request(
            "GET_SELECTED_VOICE_CHANNEL",
            json!({}),
            Kind::SelectedChannel,
            None,
        );
        nonce_of(&s, "GET_SELECTED_VOICE_CHANNEL")
    };
    s.out.clear();
    s.on_payload(reply(
        "GET_SELECTED_VOICE_CHANNEL",
        &n,
        channel("300", "General"),
    ));
    assert!(sent(&mut s).iter().all(|(c, _, _)| c != "GET_GUILD"));
    assert_eq!(s.state.history, before);
}

/// The nonce of `cmd` in an already-drained `out` list (looked up in the
/// pending map, since `sent` drops nonces).
fn nonce_of_sent(out: &[(String, Option<String>, Value)], s: &Session, cmd: &str) -> String {
    assert!(
        out.iter().any(|(c, _, _)| c == cmd),
        "no {cmd} sent: {out:?}"
    );
    s.pending
        .iter()
        .find(|(_, k)| matches!((k, cmd), (Kind::Guild(_), "GET_GUILD")))
        .map(|(n, _)| n.clone())
        .unwrap_or_else(|| panic!("no pending {cmd}"))
}

#[test]
fn rejoin_a_saved_channel_and_star_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ready_session(&dir);
    join(&mut s, "300", "General");
    s.on_command(DiscordCommand::SetFavorite("300".into(), true));
    assert!(s.state.history.is_favorite("300"), "the current channel");
    s.on_payload(dispatch(
        "VOICE_CHANNEL_SELECT",
        json!({ "channel_id": null }),
    ));
    s.out.clear();

    s.on_command(DiscordCommand::JoinChannel("999".into()));
    assert!(sent(&mut s).is_empty(), "only saved channels");
    s.on_command(DiscordCommand::JoinChannel("300".into()));
    let n = nonce_of(&s, "SELECT_VOICE_CHANNEL");
    let out = sent(&mut s);
    assert_eq!(out[0].0, "SELECT_VOICE_CHANNEL");
    assert_eq!(out[0].2, json!({ "channel_id": "300" }));
    s.on_payload(reply("SELECT_VOICE_CHANNEL", &n, channel("300", "General")));
    assert!(s.state.in_voice(), "the reply is the joined channel");
    assert!(
        sent(&mut s)
            .iter()
            .any(|(c, e, _)| c == "SUBSCRIBE" && e.as_deref() == Some("SPEAKING_START"))
    );
    // In a call, a rejoin click is ignored (Discord would refuse it).
    s.on_command(DiscordCommand::JoinChannel("300".into()));
    assert!(sent(&mut s).is_empty());
    s.on_command(DiscordCommand::SetFavorite("300".into(), false));
    assert!(!s.state.history.is_favorite("300"));
}

#[test]
fn input_volume_and_connection_status() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session_in(&dir);
    s.on_command(DiscordCommand::SetInputVolume(40));
    assert!(sent(&mut s).is_empty(), "needs ready");
    let mut s = ready_session(&dir);
    s.on_command(DiscordCommand::SetInputVolume(140));
    assert_eq!(s.state.input_volume, Some(100), "optimistic + clamped");
    let n = nonce_of(&s, "SET_VOICE_SETTINGS");
    let out = sent(&mut s);
    assert_eq!(out[0].2, json!({ "input": { "volume": 100 } }));
    s.on_payload(reply(
        "SET_VOICE_SETTINGS",
        &n,
        json!({ "input": { "device_id": "default", "volume": 87.5 }, "mute": false, "deaf": false }),
    ));
    assert_eq!(s.state.input_volume, Some(88), "Discord's answer wins");

    join(&mut s, "300", "General");
    s.on_payload(dispatch(
        "VOICE_CONNECTION_STATUS",
        json!({ "state": "VOICE_CONNECTED", "hostname": "x.discord.media", "pings": [44], "average_ping": 44.2, "last_ping": 44 }),
    ));
    assert_eq!(s.state.ping_ms, 44);
    assert!(
        GLOBAL_EVENTS.contains(&"VOICE_CONNECTION_STATUS"),
        "subscribed once authenticated"
    );
}

async fn read_raw(r: &mut UnixStream) -> (u32, Value) {
    let mut h = [0u8; 8];
    r.read_exact(&mut h).await.unwrap();
    let (op, len) = discord::decode_header(&h).unwrap();
    let mut body = vec![0u8; len as usize];
    r.read_exact(&mut body).await.unwrap();
    (op, serde_json::from_slice(&body).unwrap())
}

async fn write_raw(w: &mut UnixStream, op: u32, v: &Value) {
    w.write_all(&discord::encode_frame(op, v)).await.unwrap();
}

fn ready_event() -> Value {
    dispatch(
        "READY",
        json!({ "v": 1, "config": { "cdn_host": "cdn.discordapp.com",
                "api_endpoint": "//discord.com/api", "environment": "production" } }),
    )
}

#[tokio::test]
async fn socket_session_handshake_auth_ping_and_close() {
    let dir = tempfile::tempdir().unwrap();
    let token_file = dir.path().join("discord-token");
    save_token(&token_file, "stored");
    let (client, mut discord) = UnixStream::pair().unwrap();
    let (state_tx, mut state_rx) = watch::channel(DiscordState::default());
    let (_cmd_tx, mut cmd_rx) = mpsc::unbounded_channel();
    let client_task =
        tokio::spawn(async move { session(client, &state_tx, &mut cmd_rx, token_file).await });

    let (op, hs) = read_raw(&mut discord).await;
    assert_eq!(op, OP_HANDSHAKE);
    assert_eq!(hs, json!({ "v": 1, "client_id": STREAMKIT }));
    write_raw(&mut discord, OP_FRAME, &ready_event()).await;

    let (op, auth) = read_raw(&mut discord).await;
    assert_eq!(op, OP_FRAME);
    assert_eq!(auth["cmd"], "AUTHENTICATE");
    assert_eq!(auth["args"]["access_token"], "stored");
    state_rx
        .wait_for(|s| s.status == DiscordStatus::Authorizing)
        .await
        .unwrap();

    let nonce = auth["nonce"].as_str().unwrap().to_string();
    write_raw(&mut discord, OP_FRAME, &authenticated_reply(&nonce)).await;
    let mut cmds = Vec::new();
    for _ in 0..5 {
        cmds.push(read_raw(&mut discord).await.1["cmd"].clone());
    }
    assert_eq!(
        cmds,
        [
            "SUBSCRIBE",
            "SUBSCRIBE",
            "SUBSCRIBE",
            "GET_SELECTED_VOICE_CHANNEL",
            "GET_VOICE_SETTINGS"
        ]
    );
    let st = state_rx
        .wait_for(|s| s.status == DiscordStatus::Ready)
        .await
        .unwrap()
        .clone();
    assert_eq!(st.user_id, "10");

    write_raw(&mut discord, OP_PING, &json!({ "t": 1 })).await;
    assert_eq!(read_raw(&mut discord).await, (OP_PONG, json!({ "t": 1 })));

    write_raw(&mut discord, OP_CLOSE, &json!({ "code": 1000 })).await;
    tokio::time::timeout(Duration::from_secs(5), client_task)
        .await
        .expect("session ends on CLOSE")
        .unwrap();
}

#[tokio::test]
async fn socket_session_without_ready_gives_up() {
    let dir = tempfile::tempdir().unwrap();
    let (client, mut discord) = UnixStream::pair().unwrap();
    let (state_tx, state_rx) = watch::channel(DiscordState::default());
    let (_cmd_tx, mut cmd_rx) = mpsc::unbounded_channel();
    let token_file = dir.path().join("discord-token");
    let client_task =
        tokio::spawn(async move { session(client, &state_tx, &mut cmd_rx, token_file).await });
    read_raw(&mut discord).await;
    // Discord closes a handshake with an unknown client id.
    write_raw(
        &mut discord,
        OP_CLOSE,
        &json!({ "code": 4000, "message": "Invalid Client ID" }),
    )
    .await;
    tokio::time::timeout(Duration::from_secs(5), client_task)
        .await
        .expect("session gives up")
        .unwrap();
    assert_eq!(state_rx.borrow().status, DiscordStatus::NotRunning);
}

#[tokio::test]
async fn socket_session_without_token_waits_for_authorize() {
    let dir = tempfile::tempdir().unwrap();
    let (client, mut discord) = UnixStream::pair().unwrap();
    let (state_tx, mut state_rx) = watch::channel(DiscordState::default());
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel();
    let token_file = dir.path().join("discord-token");
    tokio::spawn(async move { session(client, &state_tx, &mut cmd_rx, token_file).await });
    read_raw(&mut discord).await;
    write_raw(&mut discord, OP_FRAME, &ready_event()).await;
    state_rx
        .wait_for(|s| s.status == DiscordStatus::NeedsAuth)
        .await
        .unwrap();
    cmd_tx.send(DiscordCommand::Authorize).unwrap();
    let (_, authz) = read_raw(&mut discord).await;
    assert_eq!(authz["cmd"], "AUTHORIZE", "first frame after READY");
    state_rx
        .wait_for(|s| s.status == DiscordStatus::Authorizing)
        .await
        .unwrap();
}
