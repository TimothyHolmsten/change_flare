# AGENTS.md

Guidance for humans and coding agents working on this repository.

## What this is

`change_flare` is a small **origin DDNS agent**, not a Cloudflare Worker. It runs on a reverse proxy / load balancer (or any NAT'd origin) and keeps Cloudflare DNS `A`/`AAAA` records equal to the host's public IP.

Do not convert it to Workers, Pages, or Wrangler unless the operator explicitly wants the updater itself to run on the edge. DNS writes still go through the [Cloudflare DNS records API](https://developers.cloudflare.com/api/resources/dns/subresources/records/methods/list/).

## Layout

| Path | Role |
| --- | --- |
| `src/main.rs` | Process entry; exits non-zero on config/startup failure |
| `src/lib.rs` | `run()`: logging, signals, health listener, poll loop |
| `src/config.rs` | 12-factor env loading; record-name matching |
| `src/ip.rs` | STUN discovery (`stunclient`, Cloudflare STUN by default) |
| `src/cloudflare.rs` | `ureq` 3 client: list `A`/`AAAA`, PATCH content only |
| `src/updater.rs` | Reconcile loop; skip Cloudflare when public IP is unchanged |
| `src/health.rs` | Optional TCP HTTP `/healthz` + `/readyz` |
| `deploy/kubernetes.yaml` | Example Deployment + Secret |
| `Dockerfile` | Multi-stage distroless image |

## Toolchain

- Edition **2024**, MSRV **1.85** (ureq 3.4). CI and `rust-toolchain.toml` use stable.
- Format: `cargo fmt --all`. Clippy: `cargo clippy --all-targets --locked -- -D warnings`.

## Invariants

- **One HTTP stack**: `ureq` 3 with a long-lived `Agent` (connection pool). Do not add `reqwest` unless async becomes a hard requirement.
- **PATCH, not PUT**: [Update DNS Record](https://developers.cloudflare.com/api/resources/dns/subresources/records/methods/edit/) (`PATCH /zones/{zone_id}/dns_records/{id}`) with `{ "content": "<ip>" }` so TTL/proxied/tags stay intact.
- **Filter server-side** with `type=A` / `type=AAAA` and paginate (`per_page=100`). Never rewrite CNAME/MX/TXT.
- **Prefer `CLOUDFLARE_RECORD_NAMES`**. Updating every address record in a zone is supported for tiny/dedicated zones only; log a warning when the filter is empty.
- **Bearer API tokens**, not Global API keys. Required permission: **Zone DNS Edit** (dashboard template: Edit zone DNS).
- **No secrets in logs, tests, or docs**. `CLOUDFLARE_API_TOKEN` / `CLOUDFLARE_API_KEY` are env-only; `.env` is gitignored.
- **Clippy `unwrap_used` / `expect_used` / `unreachable` are deny** in library code. Allow them only in `#[cfg(test)]` modules.
- **Minimum poll interval is 60s**. On STUN/API failure, log and sleep — never busy-loop.
- **Default IP mode is IPv4**. Dual-stack is opt-in (`CHANGE_FLARE_IP_MODE=both`) because many origins are v4-only.

## Tests

HTTP tests must point `CloudflareClient` / `Config.api_base` at `mockito::Server::url()`. Production URLs are `https://api.cloudflare.com/client/v4/...`; never hardcode that host in tests.

Match list queries with `mockito::Matcher::UrlEncoded("type", "A")` (not a regex on `type=A`, which also matches `AAAA`).

## Cloudflare API notes

- List: `GET /zones/{zone_id}/dns_records?type=A|AAAA&per_page=&page=`
- Edit: `PATCH /zones/{zone_id}/dns_records/{dns_record_id}`
- Auth: `Authorization: Bearer <token>`
- STUN: `stun.cloudflare.com:3478` (IPv4 and IPv6 as needed)

## Cloud-native expectations

- Config from environment (and optional `.env` for local runs).
- Container image is non-root distroless.
- Kubernetes probes hit `/healthz` (process up) and `/readyz` (at least one successful sync).
- SIGINT/SIGTERM stop the poll loop after the current sleep slice (250ms).
