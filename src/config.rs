use std::net::SocketAddr;
use std::time::Duration;

use crate::error::Error;

const DEFAULT_API_BASE: &str = "https://api.cloudflare.com/client/v4";
const DEFAULT_STUN: &str = "stun.cloudflare.com:3478";
const DEFAULT_POLL_SECS: u64 = 300;
const MIN_POLL_SECS: u64 = 60;

/// Runtime configuration loaded from the environment (12-factor).
#[derive(Clone, Debug)]
pub struct Config {
    pub api_token: String,
    pub zone_id: String,
    /// FQDNs or host labels to update. Empty means every A/AAAA in the zone.
    pub record_names: Vec<String>,
    pub poll_interval: Duration,
    pub api_base: String,
    pub stun_server: String,
    pub ipv4: bool,
    pub ipv6: bool,
    /// When true, list+reconcile Cloudflare even if the public IP is unchanged.
    pub always_reconcile: bool,
    /// Bind address for `/healthz` and `/readyz`. `None` disables the listener.
    pub health_bind: Option<SocketAddr>,
}

impl Config {
    pub fn from_env() -> Result<Self, Error> {
        let _ = dotenvy::dotenv();

        let api_token =
            first_env(&["CLOUDFLARE_API_TOKEN", "CLOUDFLARE_API_KEY"]).ok_or_else(|| {
                Error::Config("CLOUDFLARE_API_TOKEN (or CLOUDFLARE_API_KEY) is required".into())
            })?;
        if api_token.is_empty() {
            return Err(Error::Config("API token is empty".into()));
        }

        let zone_id = required("CLOUDFLARE_ZONE_ID")?;
        let record_names = env_csv("CLOUDFLARE_RECORD_NAMES");
        if record_names.is_empty() {
            log::warn!(
                "CLOUDFLARE_RECORD_NAMES is unset; every A/AAAA record in the zone will be updated"
            );
        }

        let poll_secs = env_u64("CLOUDFLARE_POLL_RATE").unwrap_or(DEFAULT_POLL_SECS);
        let poll_interval = Duration::from_secs(poll_secs.max(MIN_POLL_SECS));

        let ip_mode = std::env::var("CHANGE_FLARE_IP_MODE")
            .unwrap_or_else(|_| "ipv4".into())
            .to_ascii_lowercase();
        let (ipv4, ipv6) = match ip_mode.as_str() {
            "ipv4" => (true, false),
            "ipv6" => (false, true),
            "both" | "dual" => (true, true),
            other => {
                return Err(Error::Config(format!(
                    "CHANGE_FLARE_IP_MODE must be ipv4, ipv6, or both (got {other})"
                )));
            }
        };

        let health_bind = match std::env::var("CHANGE_FLARE_HEALTH_BIND") {
            Ok(value) if !value.is_empty() && value != "off" => Some(
                value
                    .parse()
                    .map_err(|e| Error::Config(format!("CHANGE_FLARE_HEALTH_BIND: {e}")))?,
            ),
            _ => None,
        };

        Ok(Self {
            api_token,
            zone_id,
            record_names,
            poll_interval,
            api_base: std::env::var("CLOUDFLARE_API_BASE")
                .unwrap_or_else(|_| DEFAULT_API_BASE.into()),
            stun_server: std::env::var("CLOUDFLARE_STUN_SERVER")
                .unwrap_or_else(|_| DEFAULT_STUN.into()),
            ipv4,
            ipv6,
            always_reconcile: env_truthy("CHANGE_FLARE_ALWAYS_RECONCILE"),
            health_bind,
        })
    }
}

fn required(name: &'static str) -> Result<String, Error> {
    std::env::var(name)
        .ok()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| Error::Config(format!("{name} is required")))
}

fn first_env(names: &[&'static str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| std::env::var(name).ok())
        .filter(|v| !v.is_empty())
}

fn env_csv(name: &str) -> Vec<String> {
    std::env::var(name)
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name).ok()?.parse().ok()
}

fn env_truthy(name: &str) -> bool {
    matches!(
        std::env::var(name)
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// Returns true when `wanted` refers to `record_name`.
pub fn name_matches(record_name: &str, wanted: &str) -> bool {
    let record = record_name.trim_end_matches('.').to_ascii_lowercase();
    let wanted = wanted.trim_end_matches('.').to_ascii_lowercase();
    record == wanted || record.starts_with(&format!("{wanted}."))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn name_matches_fqdn_and_label() {
        assert!(name_matches("lb.example.com", "lb.example.com"));
        assert!(name_matches("lb.example.com.", "lb"));
        assert!(name_matches("example.com", "example.com"));
        assert!(!name_matches("api.example.com", "lb"));
        assert!(!name_matches("lbl.example.com", "lb"));
    }

    #[test]
    fn poll_floor_is_sixty() {
        assert_eq!(MIN_POLL_SECS, 60);
    }
}
