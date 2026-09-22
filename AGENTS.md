# AGENTS.md

Guidance for humans and coding agents working on this repository.

## GitHub pull requests

Cursor Cloud Agent GitHub App tokens **cannot create pull requests**. `gh pr create` fails with `403 Resource not accessible by integration`. Git push still works. This is a token-scope issue, not a missing or mismatched branch.

Do this instead:

1. Commit and push the `cursor/...` branch. Do not retry `gh pr create` after a 403.
2. Open or update the PR with Cursor's **ManagePullRequest** / `open_git_pr` tool.
3. If that tool is unavailable, `.github/workflows/open-cursor-pr.yml` opens a PR on push to `cursor/**` using Actions `GITHUB_TOKEN`.

Optional (so auto-opened PRs also run CI): add a repository secret `PR_CREATE_TOKEN` — a fine-grained PAT with **Pull requests: write** and **Contents: read**. PRs created with the default `GITHUB_TOKEN` do not trigger further Actions workflows.

Optional (so `gh` inside a Cloud Agent can create PRs): add the same PAT as a Cloud Agent environment secret named `GH_TOKEN`.

## What this is

`change_flare` is a small **origin DDNS agent**, not a Cloudflare Worker. It runs on a reverse proxy / load balancer (or any NAT'd origin) and keeps Cloudflare DNS `A`/`AAAA` records equal to the host's public IP.

Do not convert it to Workers, Pages, or Wrangler unless the operator explicitly wants the updater itself to run on the edge. DNS writes still go through the [Cloudflare DNS records API](https://developers.cloudflare.com/api/resources/dns/subresources/records/methods/list/). Unrelated Workers in the same Cloudflare account are out of scope.

## Layout

| Path | Role |
| --- | --- |
| `src/main.rs` | Process entry; exits non-zero on config/startup failure |
| `src/lib.rs` | `run()`: logging, signals, health listener, poll loop |
| `src/config.rs` | 12-factor env loading; record-name matching |
| `src/ip.rs` | STUN discovery (`stunclient`, Cloudflare STUN by default); reject non-public mapped IPs |
| `src/cloudflare.rs` | `ureq` 3 client: list `A`/`AAAA`, PATCH content only, retry 429/502/503/504 |
| `src/updater.rs` | Reconcile loop; skip Cloudflare when public IP is unchanged |
| `src/health.rs` | Optional TCP HTTP `/healthz` + `/readyz` |
| `deploy/kubernetes.yaml` | Example Deployment + Secret (hostNetwork for STUN) |
| `deploy/change-flare.service` | systemd unit for a bare-metal origin |
| `deploy/compose.yaml` | Docker Compose with host networking |
| `Dockerfile` | Multi-stage distroless image |

## Toolchain

- Edition **2024**, MSRV **1.88** (`time` ≥ 0.3.47 / RUSTSEC-2026-0009). CI and `rust-toolchain.toml` use stable.
- Format: `cargo fmt --all`. Clippy: `cargo clippy --all-targets --locked -- -D warnings`.

## Invariants

- **One HTTP stack**: `ureq` 3 with a long-lived `Agent` (connection pool). `http_status_as_error` is off so 4xx/5xx JSON error bodies can be read. Do not add `reqwest` unless async becomes a hard requirement.
- **PATCH, not PUT**: [Update DNS Record](https://developers.cloudflare.com/api/resources/dns/subresources/records/methods/edit/) (`PATCH /zones/{zone_id}/dns_records/{id}`) with `{ "content": "<ip>" }` so TTL/proxied/tags stay intact.
- **Filter server-side** with `type=A` / `type=AAAA` and paginate (`per_page=100`). Request only families enabled by `CHANGE_FLARE_IP_MODE`. When every `CLOUDFLARE_RECORD_NAMES` entry is an FQDN, also pass `name=<fqdn>` (exact; trailing dots stripped). The current OpenAPI also documents nested `name.exact`; the string `name=` filter remains the compatible exact match used by cloudflare-go v4. Never rewrite CNAME/MX/TXT.
- **Prefer `CLOUDFLARE_RECORD_NAMES`**. Updating every address record in a zone is supported for tiny/dedicated zones only; log a warning when the filter is empty.
- **Bearer API tokens**, not Global API keys. Required permission: **Zone DNS Edit** (dashboard template: Edit zone DNS).
- **No secrets in logs, tests, or docs**. `CLOUDFLARE_API_TOKEN` / `CLOUDFLARE_API_KEY` are env-only; `.env` is gitignored.
- **Clippy `unwrap_used` / `expect_used` / `unreachable` are deny** in library code. Allow them only in `#[cfg(test)]` modules.
- **Minimum poll interval is 60s**. On STUN/API failure, log and sleep — never busy-loop. Retry Cloudflare `429`/`502`/`503`/`504` a few times: sleep `Retry-After` (seconds, cap 30s) or a short backoff, then retry. Do not retry without sleeping.
- **Default IP mode is IPv4**. Dual-stack is opt-in (`CHANGE_FLARE_IP_MODE=both`) because many origins are v4-only.
- **STUN results must be globally routable**. Drop loopback, RFC1918, CGNAT (`100.64/10`), link-local, unique-local, and documentation ranges so they are never PATCHed into DNS.

## Tests

HTTP tests must point `CloudflareClient` / `Config.api_base` at `mockito::Server::url()`. Production URLs are `https://api.cloudflare.com/client/v4/...`; never hardcode that host in tests.

Match list queries with `mockito::Matcher::UrlEncoded("type", "A")` (not a regex on `type=A`, which also matches `AAAA`).

## Cloudflare API notes

- List: `GET /zones/{zone_id}/dns_records?type=A|AAAA&name=&per_page=&page=`
- Edit: `PATCH /zones/{zone_id}/dns_records/{dns_record_id}`
- Auth: `Authorization: Bearer <token>`
- STUN: `stun.cloudflare.com:3478` (IPv4 and IPv6 as needed)
- Client API rate limit is 1,200 requests / 5 minutes per user; a 429 is retried.

## Cloud-native expectations

- Config from environment (and optional `.env` for local runs).
- Container image is non-root distroless. Example k8s pod uses `hostNetwork` so STUN sees the node public IP.
- Kubernetes probes hit `/healthz` (process up) and `/readyz` (at least one successful sync). `/readyz` includes `X-Last-Success-Epoch`.
- systemd unit in `deploy/change-flare.service` for hosts that are not in Kubernetes.
- Compose example: `docker compose -f deploy/compose.yaml up --build` (host network + `.env`).
- SIGINT/SIGTERM stop the poll loop after the current sleep slice (250ms).
