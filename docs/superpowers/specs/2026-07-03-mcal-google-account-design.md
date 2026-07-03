# mcal — Google account (OAuth) slice

**Date:** 2026-07-03
**Status:** Approved design, pre-implementation
**Predecessor:** [`2026-07-03-mcal-calendar-design.md`](2026-07-03-mcal-calendar-design.md) (slice 1: read-only local + remote ICS)

## Context

Slice 1 shipped `mcal` as a GTK-free calendar domain crate (ICS parse, RRULE
recurrence, local + remote-ICS providers) plus shell surfaces (clock-menu
agenda, dashboard calendar) and a `mcal today/agenda/on` CLI.

The user wants `mcal` grown into a full standalone calendar app, mirroring
dankcalendar's `dcal` (window with month/week/day/agenda views, background
daemon + tray, `account` / `sync` / `reminders` verbs, an IPC surface). That is
too large for one spec, so it is **decomposed into phases**, each its own
spec → plan → build cycle:

| Phase | Content |
|---|---|
| P1 — App shell | GTK4 window (4 views) + daemon + tray + lifecycle + `mcal ipc …` over the existing local/ICS providers |
| P2 — Accounts + cache | account registry + local sync-cache (SQLite); daemon refresh loop |
| P3 — CalDAV | `account add caldav` (iCloud/Fastmail/Nextcloud, app-password) |
| **P4 — Google OAuth** | **`account setup google` — THIS SPEC** |
| P5 — Microsoft OAuth | `account setup microsoft` |
| P6 — Reminders | reminder daemon + `mcal reminders` + notifications |

The user chose to build **Google first** (their primary calendar is a
`@compecta.com` Google Workspace account whose admin has disabled the
secret/public iCal address, so the slice-1 ICS-subscription route is a dead end
— only authenticated OAuth access reaches it).

## Goal

`mcal account setup google` connects a Google account via OAuth; its events then
appear automatically in **both** the existing `mcal today/agenda/on` CLI **and**
the slice-1 shell surfaces (clock-menu agenda, dashboard calendar), because both
call the same loader. **Read-only.** No window/tray/daemon yet (later phases).

## Locked decisions

1. **Architecture:** `mcal` becomes a standalone app (dcal 1:1), built in the
   phases above. The pure domain (parse/recur/providers) stays GTK-free; the
   window/daemon are a later layer on top.
2. **Google first**, surfaced through the existing CLI + shell (no new window in
   this slice).
3. **BYO client_id:** the user creates their own Google Cloud OAuth client
   (rclone pattern). No app verification needed — own app, self as test user.
4. **Google Calendar API v3** (REST/JSON), not CalDAV. Google expands recurrence
   server-side (`singleEvents=true`), so mcal's RRULE engine is bypassed for
   Google.
5. **mcal owns its accounts** in `~/.config/margo/mcal/`, not the shell YAML. This is
   the standalone-app model; the shell reads from mcal, not vice-versa. (Slice
   1's shell-YAML `config.calendars` for local/ICS stays as-is this slice;
   unifying it into the mcal store is a later phase.)
6. **Read-only** (`calendar.readonly` scope). Event write/create/delete is a
   later phase.

## Architecture

### 1. OAuth: loopback + PKCE

`mcal account setup google` runs Google's recommended installed-app flow:

1. Generate a PKCE `code_verifier` (32 random bytes from `/dev/urandom`,
   base64url) and `code_challenge = base64url(sha256(verifier))`; a random
   `state` for CSRF.
2. Bind a one-shot `std::net::TcpListener` on `127.0.0.1:0` (OS-assigned port);
   `redirect_uri = http://127.0.0.1:<port>`.
3. Open the browser (`xdg-open`) to
   `https://accounts.google.com/o/oauth2/v2/auth` with `response_type=code`,
   `client_id`, `redirect_uri`, `scope=…/auth/calendar.readonly`,
   `access_type=offline`, `prompt=consent`, `code_challenge`,
   `code_challenge_method=S256`, `state`.
4. The listener accepts one request, validates `state`, reads `?code=`, returns
   a tiny "you can close this tab" HTML page.
5. Exchange at `https://oauth2.googleapis.com/token`
   (`grant_type=authorization_code`, `code`, `code_verifier`, `client_id`,
   `client_secret`, `redirect_uri`) → `refresh_token` + `access_token`.
6. Store the **refresh token in the keyring**; write account metadata to the
   account store.

Token refresh (each fetch): `grant_type=refresh_token` → fresh `access_token`.
Optionally cache the access token + expiry under `~/.local/state/mcal/` to skip
a refresh when still valid (optimization, not required for MVP).

### 2. BYO credentials

`client_id` + `client_secret` live in **`~/.config/margo/mcal/credentials.toml`**:

```toml
[google]
client_id = "…apps.googleusercontent.com"
client_secret = "…"
```

If absent when `setup google` runs, mcal prints step-by-step instructions
(create a Google Cloud project → enable the Calendar API → configure the OAuth
consent screen as "External" + add yourself as a test user → create an OAuth
client of type "Desktop app" → copy id/secret here) and exits non-zero. The
installed-app `client_secret` is not treated as confidential by Google (PKCE +
loopback protect the flow); it is stored in a plain config file, the refresh
token is not.

### 3. Account store

mcal owns **`~/.config/margo/mcal/accounts.toml`** — the registry every mcal frontend
reads:

```toml
[[account]]
id = "google:kenan@compecta.com"
kind = "google"
email = "kenan@compecta.com"
display_name = "compecta"
# refresh token is in the keyring under service "mcal", user = id
```

`AccountStore::load()/save()`; `add`, `remove`, `list`. This slice only writes
`kind = "google"` rows.

### 4. Keyring

Refresh tokens go to the OS keyring via the `keyring` crate (same pattern as
`mshell-ai`), service `"mcal"`, key = the account `id`. Never written to disk in
plaintext.

### 5. GoogleProvider

A new `Provider` impl (the trait is sync; `ureq` is blocking — fits):

- `GoogleProvider::new(account, credentials)`.
- `events(window)`:
  1. Refresh → access token (keyring refresh token + credentials).
  2. `GET /calendar/v3/users/me/calendarList` → the account's calendars.
  3. For each calendar: `GET /calendar/v3/calendars/{id}/events` with
     `timeMin`/`timeMax` = window (RFC3339), `singleEvents=true`,
     `orderBy=startTime`, `maxResults=2500`, following `nextPageToken`.
  4. Map each item → `mcal::Event`: `id`, `summary`, `description`, `location`,
     `status`, `start`/`end` (`dateTime` → timed; `date` → all-day),
     `calendar_id = "google:{calendarId}"`. Recurrence already expanded, so
     `recurrence` stays empty and `recur::expand` is skipped for Google events.
- `calendars()` → the account's calendar list mapped to `mcal::Calendar`.

New deps: `serde_json` (JSON), `sha2` + `base64` (PKCE). `ureq`, `serde`,
`chrono` already present.

### 6. Unified load

`mcal::load_all` gains a third source. Today it merges local (config) + ICS
(config); it now also builds providers from the **account store** (Google this
slice) and merges them:

```
load_all(config, window) =
      local(config.local_dir)
    + ics(config.subscriptions)          // shell-owned, slice 1
    + account_store providers            // mcal-owned: Google now
```

Per-source failures are logged and skipped (existing behaviour), so a dead token
or offline network never blanks the whole agenda. Because the CLI and the shell
both call `load_all`, Google appears in both with no extra wiring. (Reading the
account store + keyring is I/O hidden inside `load_all`; documented on the fn.)

### 7. CLI

- `mcal account setup google` — the OAuth flow above.
- `mcal account list` — table of connected accounts.
- `mcal account remove <id>` — drop the account row + delete its keyring entry.
- `mcal today/agenda/on` — unchanged surface, now include Google events.

## Error handling

- Missing credentials → guided message + non-zero exit (§2).
- OAuth denied / timeout / `state` mismatch → clear stderr message, non-zero
  exit, nothing written.
- Token refresh 400 `invalid_grant` (revoked/expired refresh token) → the
  account's `events()` returns an error; `load_all` logs + skips it (agenda
  still shows other sources), and the CLI/`account list` flags it as
  "needs re-auth".
- Network/HTTP errors → per-source skip, logged.
- No `.unwrap()`/`panic!` in non-test code (panic-ratchet at baseline 370).

## Testing

- Pure/unit-testable: PKCE (verifier→challenge), auth-URL builder, account-store
  round-trip (tempfile), Google JSON→`Event` mapping (fixture responses for
  timed, all-day, cancelled, paginated, and recurring-expanded events).
- The live OAuth handshake + network calls are not unit-tested (need a real
  Google account); covered by manual verification with the user's compecta
  account.

## Out of scope (later phases)

Event write/create/delete; the standalone window + views; tray; daemon +
lifecycle (`show/toggle/run/-d/restart/kill`); the `ipc` surface; Microsoft;
CalDAV; reminders; the offline SQLite sync-cache (this slice does live fetch
with the existing disk fallback).

## Security notes

- Refresh token → keyring only. Access tokens are short-lived and kept in
  memory (or an optional state-dir cache).
- `calendar.readonly` scope — the minimum for "see my calendar"; no write, no
  Gmail/Drive.
- BYO client → no shipped secret; the installed-app `client_secret` in the
  user's own config is non-confidential per Google's installed-app model, and
  PKCE + loopback guard the exchange.

## Follow-ups / open

- Access-token caching (state dir) — optimization, deferred.
- Which calendars to include (all vs a selectable subset) — this slice pulls
  **all** calendars in the account; per-calendar toggles can come with the P1
  window's settings.
- Re-auth UX (`mcal account setup google` re-run overwrites) — fine for CLI;
  the P1 window can add a friendlier prompt.
