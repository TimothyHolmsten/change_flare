# change_flare

Keeps Cloudflare DNS `A` and `AAAA` records pointed at **this host's public IP**. Typical placement is a reverse proxy, load balancer, or other origin that sits behind NAT and needs Cloudflare to track address changes.

Each poll:

1. Discovers the public IPv4 and/or IPv6 address with STUN (`stun.cloudflare.com:3478`).
2. Lists only `A`/`AAAA` records in the configured zone (server-side type filter, paginated).
3. PATCHes record **content** when it differs. Unchanged records and unchanged public IPs skip the write path.

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
| `CLOUDFLARE_RECORD_NAMES` | recommended | *(all A/AAAA)* | Comma-separated names (`lb.example.com` or host label `lb`) |
| `CLOUDFLARE_POLL_RATE` | no | `300` | Seconds between polls (minimum `60`) |
| `CHANGE_FLARE_IP_MODE` | no | `ipv4` | `ipv4`, `ipv6`, or `both` |
| `CLOUDFLARE_STUN_SERVER` | no | `stun.cloudflare.com:3478` | STUN host:port |
| `CLOUDFLARE_API_BASE` | no | `https://api.cloudflare.com/client/v4` | Override for tests |
| `CHANGE_FLARE_ALWAYS_RECONCILE` | no | off | List Cloudflare even when the public IP has not changed |
| `CHANGE_FLARE_HEALTH_BIND` | no | off | Bind `host:port` for `/healthz` and `/readyz` |
| `RUST_LOG` | no | `info` | `log` / `env_logger` filter |

Do not commit `.env`. Tokens are never logged.

## Run on a node

Systemd, Docker, or Kubernetes all work. The process is a single blocking loop; send `SIGINT`/`SIGTERM` to drain and exit.

```bash
docker build -t change_flare .
docker run --rm --env-file .env change_flare
```

Kubernetes manifests live in `deploy/kubernetes.yaml`. Set `CHANGE_FLARE_HEALTH_BIND=0.0.0.0:8080` so kubelet can probe `/healthz` (liveness) and `/readyz` (ready after the first successful sync).

## Develop

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

Live STUN against the network is not part of the default test suite.
