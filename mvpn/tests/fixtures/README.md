# mvpn test fixtures

Real `mullvad` output captured on the maintainer's machine (mullvad-cli
2026.5), trimmed and redacted: IP addresses are documentation addresses
(203.0.113.0/24), coordinates of the real location are made up, and no
account number appears anywhere. Files named `synthetic-*` reproduce the
exact CLI text for states that cannot be captured safely (daemon down,
error states) from mullvad-cli's own strings.

- `relay-list.txt` — `mullvad relay list`, seven countries, two relays a city.
- `relay-get.txt` — `mullvad relay get` (multihop off, a pinned relay).
- `relay-get-multihop.txt` — `mullvad relay get` with multihop on, from the
  community `mullvad` plugin's own captures (same CLI version): turning
  multihop on here would change the live tunnel.
- `status-connected.json` — `mullvad status --json` while connected; addresses replaced with documentation addresses, coordinates set to the relay city's.
