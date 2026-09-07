use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use crate::cloudflare::{CloudflareClient, DnsRecord};
use crate::config::Config;
use crate::error::Error;
use crate::health::HealthState;
use crate::ip::{self, PublicIps};

/// Polls STUN and reconciles Cloudflare DNS records.
pub struct Updater {
    config: Config,
    client: CloudflareClient,
    health: HealthState,
    last_ips: Option<PublicIps>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub examined: usize,
    pub updated: usize,
    pub skipped: usize,
}

impl Updater {
    pub fn new(config: Config, health: HealthState) -> Self {
        let client = CloudflareClient::new(
            config.api_base.clone(),
            config.api_token.clone(),
            config.zone_id.clone(),
        );
        Self {
            config,
            client,
            health,
            last_ips: None,
        }
    }

    pub fn run_until(&mut self, running: &AtomicBool) -> Result<(), Error> {
        while running.load(Ordering::SeqCst) {
            match self.tick() {
                Ok(report) => {
                    log::info!(
                        "sync examined={} updated={} skipped={}",
                        report.examined,
                        report.updated,
                        report.skipped
                    );
                    self.health.mark_success();
                }
                Err(err) => log::error!("{err}"),
            }

            if !interruptible_sleep(self.config.poll_interval, running) {
                break;
            }
        }
        log::info!("shutting down");
        Ok(())
    }

    pub fn tick(&mut self) -> Result<SyncReport, Error> {
        let ips = ip::discover(&self.config.stun_server, self.config.ipv4, self.config.ipv6)?;
        self.reconcile(ips)
    }

    /// Reconcile records against already-discovered IPs (used by tests).
    pub fn reconcile(&mut self, ips: PublicIps) -> Result<SyncReport, Error> {
        if ips.is_empty() {
            return Err(Error::Stun("no public IP discovered".into()));
        }

        if !self.config.always_reconcile && self.last_ips.as_ref() == Some(&ips) {
            log::debug!("public IP unchanged; skipping Cloudflare API");
            return Ok(SyncReport::default());
        }

        log::info!("public IPs v4={:?} v6={:?}", ips.v4, ips.v6);
        let records = self
            .client
            .list_address_records(&self.config.record_names, &self.config.dns_types())?;
        let report = apply_updates(&self.client, &records, &ips)?;
        self.last_ips = Some(ips);
        Ok(report)
    }
}

/// Update records whose content differs from the discovered address of the same family.
pub fn apply_updates(
    client: &CloudflareClient,
    records: &[DnsRecord],
    ips: &PublicIps,
) -> Result<SyncReport, Error> {
    let mut report = SyncReport {
        examined: records.len(),
        ..SyncReport::default()
    };

    for record in records {
        let Some(desired) = ips.for_record_type(&record.record_type) else {
            log::debug!(
                "skip {} {} (no {} address discovered)",
                record.record_type,
                record.name,
                record.record_type
            );
            report.skipped += 1;
            continue;
        };

        if record_already_current(&record.content, desired) {
            report.skipped += 1;
            continue;
        }

        log::info!(
            "updating {} {} {} -> {desired}",
            record.record_type,
            record.name,
            record.content
        );
        client.patch_content(&record.id, desired)?;
        report.updated += 1;
    }

    Ok(report)
}

fn record_already_current(content: &str, desired: IpAddr) -> bool {
    content
        .parse::<IpAddr>()
        .map(|current| current == desired)
        .unwrap_or(false)
}

fn interruptible_sleep(total: Duration, running: &AtomicBool) -> bool {
    let slice = Duration::from_millis(250);
    let mut remaining = total;
    while remaining > Duration::ZERO {
        if !running.load(Ordering::SeqCst) {
            return false;
        }
        let step = remaining.min(slice);
        thread::sleep(step);
        remaining = remaining.saturating_sub(step);
    }
    running.load(Ordering::SeqCst)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::cloudflare::CloudflareClient;
    use std::net::{Ipv4Addr, Ipv6Addr};

    fn list_body(records: serde_json::Value) -> String {
        serde_json::json!({
            "success": true,
            "errors": [],
            "result": records,
            "result_info": { "page": 1, "per_page": 100, "total_pages": 1 }
        })
        .to_string()
    }

    fn test_config(api_base: String) -> Config {
        Config {
            api_token: "token".into(),
            zone_id: "zone1".into(),
            record_names: vec!["lb".into()],
            poll_interval: Duration::from_secs(60),
            api_base,
            stun_server: "stun.example:3478".into(),
            ipv4: true,
            ipv6: false,
            always_reconcile: true,
            health_bind: None,
        }
    }

    #[test]
    fn skips_patch_when_content_matches() {
        let records = [DnsRecord {
            id: "rec-a".into(),
            name: "lb.example.com".into(),
            record_type: "A".into(),
            content: "203.0.113.10".into(),
        }];
        let ips = PublicIps {
            v4: Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10))),
            v6: None,
        };
        // No HTTP server: apply_updates must not call patch.
        let client = CloudflareClient::new("http://127.0.0.1:1", "token", "zone1");
        let report = apply_updates(&client, &records, &ips).unwrap();
        assert_eq!(
            report,
            SyncReport {
                examined: 1,
                updated: 0,
                skipped: 1
            }
        );
    }

    #[test]
    fn patches_when_ip_changed() {
        let mut server = mockito::Server::new();
        let patch = server
            .mock("PATCH", "/zones/zone1/dns_records/rec-a")
            .match_body(mockito::Matcher::JsonString(
                r#"{"content":"198.51.100.7"}"#.into(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "success": true,
                    "result": {
                        "id": "rec-a",
                        "name": "lb.example.com",
                        "type": "A",
                        "content": "198.51.100.7"
                    }
                })
                .to_string(),
            )
            .create();

        let client = CloudflareClient::new(server.url(), "token", "zone1");
        let records = [DnsRecord {
            id: "rec-a".into(),
            name: "lb.example.com".into(),
            record_type: "A".into(),
            content: "203.0.113.10".into(),
        }];
        let ips = PublicIps {
            v4: Some(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7))),
            v6: None,
        };
        let report = apply_updates(&client, &records, &ips).unwrap();
        patch.assert();
        assert_eq!(report.updated, 1);
    }

    #[test]
    fn skips_aaaa_without_ipv6() {
        let client = CloudflareClient::new("http://127.0.0.1:1", "token", "zone1");
        let records = [DnsRecord {
            id: "rec-aaaa".into(),
            name: "lb.example.com".into(),
            record_type: "AAAA".into(),
            content: "::1".into(),
        }];
        let ips = PublicIps {
            v4: Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10))),
            v6: None,
        };
        let report = apply_updates(&client, &records, &ips).unwrap();
        assert_eq!(report.skipped, 1);
        assert_eq!(report.updated, 0);
    }

    #[test]
    fn reconcile_lists_then_patches() {
        let mut server = mockito::Server::new();
        let a = server
            .mock("GET", "/zones/zone1/dns_records")
            .match_query(mockito::Matcher::UrlEncoded("type".into(), "A".into()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(list_body(serde_json::json!([{
                "id": "rec-a",
                "name": "lb.example.com",
                "type": "A",
                "content": "203.0.113.10"
            }])))
            .create();
        let patch = server
            .mock("PATCH", "/zones/zone1/dns_records/rec-a")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "success": true,
                    "result": {
                        "id": "rec-a",
                        "name": "lb.example.com",
                        "type": "A",
                        "content": "198.51.100.7"
                    }
                })
                .to_string(),
            )
            .create();

        let mut updater = Updater::new(test_config(server.url()), HealthState::new());
        let report = updater
            .reconcile(PublicIps {
                v4: Some(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7))),
                v6: None,
            })
            .unwrap();

        a.assert();
        patch.assert();
        assert_eq!(report.updated, 1);
    }

    #[test]
    fn skips_cloudflare_when_ip_unchanged() {
        let mut server = mockito::Server::new();
        let a = server
            .mock("GET", "/zones/zone1/dns_records")
            .match_query(mockito::Matcher::UrlEncoded("type".into(), "A".into()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(list_body(serde_json::json!([{
                "id": "rec-a",
                "name": "lb.example.com",
                "type": "A",
                "content": "203.0.113.10"
            }])))
            .expect(1)
            .create();

        let mut cfg = test_config(server.url());
        cfg.always_reconcile = false;
        let mut updater = Updater::new(cfg, HealthState::new());
        let ips = PublicIps {
            v4: Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10))),
            v6: None,
        };
        updater.reconcile(ips.clone()).unwrap();
        let second = updater.reconcile(ips).unwrap();
        a.assert();
        assert_eq!(second, SyncReport::default());
    }

    #[test]
    fn updates_aaaa_when_v6_present() {
        let mut server = mockito::Server::new();
        let patch = server
            .mock("PATCH", "/zones/zone1/dns_records/rec-aaaa")
            .match_body(mockito::Matcher::JsonString(
                r#"{"content":"2001:db8::2"}"#.into(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "success": true,
                    "result": {
                        "id": "rec-aaaa",
                        "name": "lb.example.com",
                        "type": "AAAA",
                        "content": "2001:db8::2"
                    }
                })
                .to_string(),
            )
            .create();
        let client = CloudflareClient::new(server.url(), "token", "zone1");
        let records = [DnsRecord {
            id: "rec-aaaa".into(),
            name: "lb.example.com".into(),
            record_type: "AAAA".into(),
            content: "2001:db8::1".into(),
        }];
        let ips = PublicIps {
            v4: None,
            v6: Some(IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 2))),
        };
        let report = apply_updates(&client, &records, &ips).unwrap();
        patch.assert();
        assert_eq!(report.updated, 1);
    }
}
