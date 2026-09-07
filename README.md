# change_flare

Keeps Cloudflare DNS `A` and `AAAA` records pointed at **this host's public IP**. Typical placement is a reverse proxy, load balancer, or other origin that sits behind NAT and needs Cloudflare to track address changes.

This is an origin process, not a Cloudflare Worker. DNS writes go through the [Cloudflare DNS records API](https://developers.cloudflare.com/api/resources/dns/subresources/records/methods/list/).

Each poll:

1. Discovers the public IPv4 and/or IPv6 address with STUN (`stun.cloudflare.com:3478`). Mapped addresses that are not globally routable (private, loopback, CGNAT, documentation) are discarded.
2. Lists only the record types that match `CHANGE_FLARE_IP_MODE` (`A`, `AAAA`, or both). FQDNs in `CLOUDFLARE_RECORD_NAMES` are filtered server-side with `name` (trailing dots stripped); host labels are matched locally.
3. PATCHes record **content** when it differs. Unchanged records and unchanged public IPs skip the write path. Transient Cloudflare `429`/`5xx` responses are retried.

## Setup

1. Create an API token in the Cloudflare dashboard using the **Edit zone DNS** template (permission: **Zone / DNS / Edit**). Scope it to the one zone this node should update.
2. Copy `.env.example` to `.env` and fill in values (or set the same variables in the process environment / container secret).

```bash
cp .env.example .env
cargo run --release
```

## Environment

| Variable | Required | Default | Purpose |
| --- | --- | --- | --- |
| `CLOUDFLARE_API_TOKEN` | yes | — | API token (`CLOUDFLARE_API_KEY` is accepted as a legacy alias) |
| `CLOUDFLARE_ZONE_ID` | yes | — | Zone identifier |
| `CLOUDFLARE_RECORD_NAMES` | recommended | *(all A/AAAA of the selected families)* | Comma-separated names (`lb.example.com` or host label `lb`) |
| `CLOUDFLARE_POLL_RATE` | no | `300` | Seconds between polls (minimum `60`) |
| `CHANGE_FLARE_IP_MODE` | no | `ipv4` | `ipv4`, `ipv6`, or `both` |
| `CLOUDFLARE_STUN_SERVER` | no | `stun.cloudflare.com:3478` | STUN host:port |
| `CLOUDFLARE_API_BASE` | no | `https://api.cloudflare.com/client/v4` | Override for tests |
| `CHANGE_FLARE_ALWAYS_RECONCILE` | no | off | List Cloudflare even when the public IP has not changed |
| `CHANGE_FLARE_HEALTH_BIND` | no | off | Bind `host:port` for `/healthz` and `/readyz` |
| `RUST_LOG` | no | `info` | `log` / `env_logger` filter |

Do not commit `.env`. Tokens are never logged.

## Run on a node

The process is a single blocking loop; send `SIGINT`/`SIGTERM` to drain and exit.

### Docker

```bash
docker build -t change_flare .
docker run --rm --network host --env-file .env change_flare
```

`--network host` is important: STUN must observe the node's public address, not a container NAT.

Compose equivalent (reads `.env` from the repo root):

```bash
docker compose -f deploy/compose.yaml up --build
```

### systemd

Install the binary, copy `deploy/change-flare.service`, and put secrets in `/etc/change_flare.env` (same keys as `.env.example`).

```bash
sudo install -m 0755 target/release/change_flare /usr/local/bin/change_flare
sudo install -m 0644 deploy/change-flare.service /etc/systemd/system/change-flare.service
sudo install -m 0600 .env /etc/change_flare.env
sudo systemctl enable --now change-flare
```

### Kubernetes

Manifests live in `deploy/kubernetes.yaml`. The example uses `hostNetwork` so STUN sees the node's public IP, `dnsPolicy: ClusterFirstWithHostNet` so `api.cloudflare.com` still resolves, and `CHANGE_FLARE_HEALTH_BIND=0.0.0.0:8080` so kubelet can probe `/healthz` (startup + liveness) and `/readyz` (ready after the first successful sync). Keep `replicas: 1` for a given DNS name; multiple writers would race.

## Develop

```bash
cargo fmt --all
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
```

Live STUN against the network is not part of the default test suite. Agent notes for this repo are in `AGENTS.md`.
