# Direct HTTPS

Browsers reach a registered server directly at

```
https://<ip-encoded>.<label>.servers.redseat.cloud:<port>
```

and fall back to WebRTC when no address answers. Design: redseat-svelte issue #62. The cloud-side
reference is `docs/direct-https.md` in the SvelteKit repo.

- `<label>` is a random per-server label assigned by the cloud (not the server id).
- `<ip-encoded>` is `a-b-c-d` for IPv4 and the RFC 5952 IPv6 form with `-` for `:`. The server never
  parses it: one wildcard certificate `*.<label>.servers.redseat.cloud` covers every address.
- The cloud runs ACME (Let's Encrypt, DNS-01). The server generates its ECDSA P-256 key locally and
  only sends a CSR. The private key never leaves the server.

Code: `src/direct/` (`mod.rs` background loop, `cloud.rs` API client, `addresses.rs` discovery and
UPnP, `tls.rs` SNI resolver and CSR).

## When it runs

For registered servers (`id` and `token` in `config.json`), unless `domain` is set or `noCert` is
true: those setups are unchanged. The legacy `<id>-srv.redseat.cloud` certificate is still obtained
and served alongside the direct one until every client has moved; failing to get it no longer stops
the server.

## Custom domain report

Every registered server reports its custom domain to `PATCH /api/servers/<id>` (registration
token) in the background at startup. The domain only changes with the config, which is read at
startup, so a failed report is retried with backoff (1 min, doubling up to 1 h) until it succeeds,
then re-sent daily in case the cloud's copy was lost.

- `{ "domain": "nseat.example.org" }`, or `{ "domain": "nseat.example.org", "port": 8443 }` when the
  public port isn't 443. The port comes from the domain value (`REDSEAT_DOMAIN=host:8443`) or from
  `REDSEAT_EXP_PORT`/`exp_port`, never from the listening port: behind a reverse proxy (Traefik…) the
  container's port isn't what clients connect to.
- `{ "domain": null }` when none is set, so the cloud clears a stale one.

The web app tries `https://<domain>[:<port>]/ping` like a direct candidate. The server no longer
reports its public IPv4 for the legacy `<id>-srv.redseat.cloud` record: that name stops following
IP changes.

## Background loop

All calls use `Authorization: Token <registration token>` on `https://<home>/api/servers/<id>`.

| What | Endpoint | When |
|---|---|---|
| Candidate addresses | `PATCH /api/servers/<id>` `{ lan, ipv4?, ipv6, port }` | Startup, every 10 min when they changed, and daily |
| Authorized users | `PUT …/members` `{ users: [uid…] }` (every server user) | Startup, whenever a user is added, and daily |
| Certificate status | `GET …/certificate` | Startup, daily, every 5 min while an order is queued or processing |
| Certificate request | `POST …/certificate` `{ csr }` | Only when `csrNeeded` is true |

- **Addresses:** LAN addresses are the private IPv4 and unique-local IPv6 addresses of up interfaces
  (container and VM bridges such as `docker0` and `br-*` are skipped). Global IPv6 addresses go in `ipv6`.
  The public IPv4 is only reported when the port is reachable on it: the interface itself has a public
  address, a UPnP-IGD mapping succeeded, or the port is declared as forwarded manually (see below).
  The UPnP mapping (`RedSeat`, 1 h lease) is renewed on every check, so it expires within an hour once RedSeat stops. Routers that only allow permanent mappings get none: forward the port manually and set `portForwarded`.
- **Certificate install:** when `certificate.label` equals `label` and `certificate.notBefore` changed
  (or no certificate for that label could be loaded from disk at startup), the chain is matched against the pending keys (keys of CSRs sent but not issued yet) and the current key.
  TLS is hot-reloaded: no restart.
- **Responses to `POST …/certificate`:** `409` with an order id means an order is already running,
  so the loop keeps polling. `409`/`429`/`400`/`503` with a message mean the server waits for the next
  daily check instead of retrying.
- **Label rotation:** during a rotation, the certificate for the old label keeps being served for its
  names until the process restarts, and the new one is served for the new label.
- **Lost key:** if the issued certificate matches no local key (for example after restoring a config
  folder without it), the server rotates its label once so the cloud asks for a new CSR.

## TLS

The listener serves HTTP and HTTPS on the same port. The certificate is chosen from the SNI:

- `*.<label>.servers.redseat.cloud`: the direct certificate (and the previous label's during a rotation);
- `<id>-srv.redseat.cloud` / `*.<id>-srv.redseat.cloud`: the legacy certificate.

Handshakes without SNI, with a bare IP, or with any other name get no certificate and fail. This stops
internet-wide scanners from linking the server's IP to its label.

## HTTP

- `GET /ping` answers 2xx for the client's probe.
- CORS allows any origin on every route, echoes the requested headers (`Authorization`, `SHARETOKEN`,
  `Range`…), and exposes `Content-Range`, `Accept-Ranges`, `Content-Length`, `Content-Disposition` and `Content-Type`.
- Preflights with `Access-Control-Request-Private-Network: true` get `Access-Control-Allow-Private-Network: true`,
  so the public web app can call LAN addresses in Chromium.

## Configuration

| `config.json` | Environment | Purpose |
|---|---|---|
| `portForwarded` | `REDSEAT_PORT_FORWARDED=true` | The port is forwarded manually on the router: report the public IPv4 without UPnP. |
| `lanIps` | `REDSEAT_LAN_IPS=192.168.1.10,fd00::10` | LAN addresses to report instead of the discovered ones (for example the host's addresses in Docker). UPnP then maps to the first of these IPv4 addresses. |
| `exp_port` | `REDSEAT_EXP_PORT` | Port clients connect to, when it differs from the listening port. Also the custom domain's port when the domain value has none. |
| `domain` | `REDSEAT_DOMAIN=host[:port]` | Custom domain (TLS handled in front of the server): disables direct HTTPS, reported to the cloud. |

The registration redirect (`/infos/register`) accepts an optional `label` query parameter. Servers
registered without it get the label from `GET …/certificate` on first run.

## Files (config folder)

| File | Content |
|---|---|
| `direct_https.json` | Label, pending label, installed certificate `notBefore`/label, name of the last CSR |
| `direct_cert_chain.pem` / `direct_cert_key.pem` | Installed chain and its private key |
| `direct_pending_keys.json` | Keys of CSRs sent but not issued yet (newest first, at most 4) |
