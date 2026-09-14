use std::net::IpAddr;
use std::thread;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use ureq::Agent;

use crate::config::name_matches;
use crate::error::Error;

const USER_AGENT: &str = concat!("change_flare/", env!("CARGO_PKG_VERSION"));
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);
const PAGE_SIZE: &str = "100";
const MAX_ATTEMPTS: u32 = 3;
const MAX_RETRY_WAIT: Duration = Duration::from_secs(30);

/// Cloudflare DNS client. Reuses a single `ureq::Agent` (connection pool).
pub struct CloudflareClient {
    agent: Agent,
    api_base: String,
    authorization: String,
    zone_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DnsRecord {
    pub id: String,
    pub name: String,
    pub record_type: String,
    pub content: String,
}

#[derive(Deserialize)]
struct ApiResponse<T> {
    success: bool,
    result: Option<T>,
    errors: Option<Vec<ApiError>>,
    result_info: Option<ResultInfo>,
}

#[derive(Deserialize)]
struct ApiError {
    message: Option<String>,
}

#[derive(Deserialize)]
struct ResultInfo {
    total_pages: Option<u32>,
}

#[derive(Deserialize)]
struct RawRecord {
    id: String,
    name: String,
    #[serde(rename = "type")]
    record_type: String,
    content: String,
}

#[derive(Serialize)]
struct PatchBody<'a> {
    content: &'a str,
}

impl CloudflareClient {
    pub fn new(
        api_base: impl Into<String>,
        token: impl Into<String>,
        zone_id: impl Into<String>,
    ) -> Self {
        let agent: Agent = Agent::config_builder()
            .timeout_global(Some(HTTP_TIMEOUT))
            .http_status_as_error(false)
            .build()
            .into();

        Self {
            agent,
            api_base: api_base.into().trim_end_matches('/').to_string(),
            authorization: format!("Bearer {}", token.into()),
            zone_id: zone_id.into(),
        }
    }

    /// List address records for the requested types (`A` / `AAAA`).
    ///
    /// FQDNs in `record_names` are filtered server-side with `name` (exact).
    /// Host labels still list the type and match locally.
    pub fn list_address_records(
        &self,
        record_names: &[String],
        types: &[&str],
    ) -> Result<Vec<DnsRecord>, Error> {
        let mut records = Vec::new();
        let exact_names = exact_fqdn_filters(record_names);

        for record_type in types {
            if let Some(names) = exact_names.as_deref() {
                for name in names {
                    records.extend(self.list_type(record_type, Some(name))?);
                }
            } else {
                records.extend(self.list_type(record_type, None)?);
            }
        }

        if record_names.is_empty() {
            return Ok(records);
        }

        Ok(records
            .into_iter()
            .filter(|record| {
                record_names
                    .iter()
                    .any(|wanted| name_matches(&record.name, wanted))
            })
            .collect())
    }

    fn list_type(&self, record_type: &str, name: Option<&str>) -> Result<Vec<DnsRecord>, Error> {
        let mut page = 1u32;
        let mut records = Vec::new();
        let list_url = format!("{}/zones/{}/dns_records", self.api_base, self.zone_id);

        loop {
            let parsed: ApiResponse<Vec<RawRecord>> = self.send_json(|| {
                let mut request = self
                    .agent
                    .get(&list_url)
                    .header("Authorization", &self.authorization)
                    .header("User-Agent", USER_AGENT)
                    .query("type", record_type)
                    .query("per_page", PAGE_SIZE)
                    .query("page", page.to_string());
                if let Some(name) = name {
                    request = request.query("name", name);
                }
                request.call()
            })?;

            let page_records = parsed.result.unwrap_or_default();
            let count = page_records.len();
            records.extend(page_records.into_iter().map(|raw| DnsRecord {
                id: raw.id,
                name: raw.name,
                record_type: raw.record_type,
                content: raw.content,
            }));

            let total_pages = parsed
                .result_info
                .and_then(|info| info.total_pages)
                .unwrap_or(1);
            if page >= total_pages || count == 0 {
                break;
            }
            page += 1;
        }

        Ok(records)
    }

    /// PATCH only the record content (Cloudflare partial update).
    pub fn patch_content(&self, record_id: &str, content: IpAddr) -> Result<(), Error> {
        let url = format!(
            "{}/zones/{}/dns_records/{record_id}",
            self.api_base, self.zone_id
        );
        let content_str = content.to_string();
        let body = PatchBody {
            content: &content_str,
        };
        self.send_json::<RawRecord>(|| {
            self.agent
                .patch(&url)
                .header("Authorization", &self.authorization)
                .header("User-Agent", USER_AGENT)
                .header("Content-Type", "application/json")
                .send_json(&body)
        })?;
        Ok(())
    }

    fn send_json<T: DeserializeOwned>(
        &self,
        mut call: impl FnMut() -> Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> Result<ApiResponse<T>, Error> {
        let mut last_status = 0u16;
        for attempt in 1..=MAX_ATTEMPTS {
            let mut response = match call() {
                Ok(response) => response,
                Err(err) if attempt < MAX_ATTEMPTS => {
                    log::warn!(
                        "Cloudflare transport error (attempt {attempt}/{MAX_ATTEMPTS}): {err}"
                    );
                    thread::sleep(backoff(attempt));
                    continue;
                }
                Err(err) => return Err(err.into()),
            };

            let status = response.status().as_u16();
            last_status = status;
            if retryable_status(status) && attempt < MAX_ATTEMPTS {
                let wait =
                    retry_after_delay(response.headers()).unwrap_or_else(|| backoff(attempt));
                log::warn!(
                    "Cloudflare HTTP {status} (attempt {attempt}/{MAX_ATTEMPTS}); retrying in {wait:?}"
                );
                drop(response);
                thread::sleep(wait);
                continue;
            }

            let parsed: Result<ApiResponse<T>, _> = response.body_mut().read_json();
            match parsed {
                Ok(body) if (200..300).contains(&status) && body.success => return Ok(body),
                Ok(body) => return Err(api_error(body.errors)),
                Err(_) if !(200..300).contains(&status) => {
                    return Err(Error::Http(format!("HTTP {status}")));
                }
                Err(err) => return Err(err.into()),
            }
        }
        Err(Error::Http(format!("HTTP {last_status}")))
    }
}

/// Cloudflare `name` is an exact FQDN match. Host labels still need a full list.
fn exact_fqdn_filters(record_names: &[String]) -> Option<Vec<String>> {
    if record_names.is_empty() {
        return None;
    }
    if !record_names.iter().all(|name| name.contains('.')) {
        return None;
    }
    Some(
        record_names
            .iter()
            .map(|name| name.trim_end_matches('.').to_ascii_lowercase())
            .collect(),
    )
}

fn retryable_status(status: u16) -> bool {
    matches!(status, 429 | 502 | 503 | 504)
}

fn backoff(attempt: u32) -> Duration {
    if cfg!(test) {
        Duration::from_millis(1)
    } else {
        Duration::from_millis(200 * u64::from(attempt))
    }
}

fn retry_after_delay(headers: &ureq::http::HeaderMap) -> Option<Duration> {
    let value = headers.get("retry-after")?.to_str().ok()?;
    let secs: u64 = value.parse().ok()?;
    Some(Duration::from_secs(secs).min(MAX_RETRY_WAIT))
}

fn api_error(errors: Option<Vec<ApiError>>) -> Error {
    let message = errors
        .into_iter()
        .flatten()
        .filter_map(|e| e.message)
        .collect::<Vec<_>>()
        .join("; ");
    Error::Api(if message.is_empty() {
        "request failed".into()
    } else {
        message
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn list_body(records: serde_json::Value) -> String {
        serde_json::json!({
            "success": true,
            "errors": [],
            "result": records,
            "result_info": { "page": 1, "per_page": 100, "total_pages": 1 }
        })
        .to_string()
    }

    #[test]
    fn lists_and_filters_records() {
        let mut server = mockito::Server::new();
        let a = server
            .mock("GET", "/zones/zone1/dns_records")
            .match_query(mockito::Matcher::UrlEncoded("type".into(), "A".into()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(list_body(serde_json::json!([
                {
                    "id": "rec-a",
                    "name": "lb.example.com",
                    "type": "A",
                    "content": "203.0.113.10"
                },
                {
                    "id": "rec-other",
                    "name": "api.example.com",
                    "type": "A",
                    "content": "203.0.113.20"
                }
            ])))
            .create();
        let aaaa = server
            .mock("GET", "/zones/zone1/dns_records")
            .match_query(mockito::Matcher::UrlEncoded("type".into(), "AAAA".into()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(list_body(serde_json::json!([])))
            .create();

        let client = CloudflareClient::new(server.url(), "token", "zone1");
        let records = client
            .list_address_records(&["lb".to_string()], &["A", "AAAA"])
            .unwrap();

        a.assert();
        aaaa.assert();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, "rec-a");
        assert_eq!(records[0].content, "203.0.113.10");
    }

    #[test]
    fn patches_content_only() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("PATCH", "/zones/zone1/dns_records/rec-a")
            .match_header("authorization", "Bearer token")
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
        client
            .patch_content("rec-a", IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7)))
            .unwrap();
        mock.assert();
    }

    #[test]
    fn surfaces_api_errors() {
        let mut server = mockito::Server::new();
        let _mock = server
            .mock("GET", "/zones/zone1/dns_records")
            .match_query(mockito::Matcher::UrlEncoded("type".into(), "A".into()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "success": false,
                    "errors": [{ "message": "Authentication error" }],
                    "result": null
                })
                .to_string(),
            )
            .create();

        let client = CloudflareClient::new(server.url(), "token", "zone1");
        let err = client.list_type("A", None).unwrap_err();
        assert!(err.to_string().contains("Authentication error"));
    }

    #[test]
    fn lists_only_requested_types() {
        let mut server = mockito::Server::new();
        let a = server
            .mock("GET", "/zones/zone1/dns_records")
            .match_query(mockito::Matcher::UrlEncoded("type".into(), "A".into()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(list_body(serde_json::json!([])))
            .create();
        let aaaa = server
            .mock("GET", "/zones/zone1/dns_records")
            .match_query(mockito::Matcher::UrlEncoded("type".into(), "AAAA".into()))
            .expect(0)
            .create();

        let client = CloudflareClient::new(server.url(), "token", "zone1");
        let records = client.list_address_records(&[], &["A"]).unwrap();
        a.assert();
        aaaa.assert();
        assert!(records.is_empty());
    }

    #[test]
    fn sends_exact_name_for_fqdn() {
        let mut server = mockito::Server::new();
        let a = server
            .mock("GET", "/zones/zone1/dns_records")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("type".into(), "A".into()),
                mockito::Matcher::UrlEncoded("name".into(), "lb.example.com".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(list_body(serde_json::json!([{
                "id": "rec-a",
                "name": "lb.example.com",
                "type": "A",
                "content": "203.0.113.10"
            }])))
            .create();

        let client = CloudflareClient::new(server.url(), "token", "zone1");
        let records = client
            .list_address_records(&["lb.example.com".to_string()], &["A"])
            .unwrap();
        a.assert();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, "rec-a");
    }

    #[test]
    fn paginates_list_results() {
        let mut server = mockito::Server::new();
        let page1 = server
            .mock("GET", "/zones/zone1/dns_records")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("type".into(), "A".into()),
                mockito::Matcher::UrlEncoded("page".into(), "1".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "success": true,
                    "errors": [],
                    "result": [{
                        "id": "rec-1",
                        "name": "a.example.com",
                        "type": "A",
                        "content": "203.0.113.1"
                    }],
                    "result_info": { "page": 1, "per_page": 100, "total_pages": 2 }
                })
                .to_string(),
            )
            .create();
        let page2 = server
            .mock("GET", "/zones/zone1/dns_records")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("type".into(), "A".into()),
                mockito::Matcher::UrlEncoded("page".into(), "2".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "success": true,
                    "errors": [],
                    "result": [{
                        "id": "rec-2",
                        "name": "b.example.com",
                        "type": "A",
                        "content": "203.0.113.2"
                    }],
                    "result_info": { "page": 2, "per_page": 100, "total_pages": 2 }
                })
                .to_string(),
            )
            .create();

        let client = CloudflareClient::new(server.url(), "token", "zone1");
        let records = client.list_address_records(&[], &["A"]).unwrap();
        page1.assert();
        page2.assert();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].id, "rec-1");
        assert_eq!(records[1].id, "rec-2");
    }

    #[test]
    fn strips_trailing_dot_on_fqdn_filter() {
        let mut server = mockito::Server::new();
        let a = server
            .mock("GET", "/zones/zone1/dns_records")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("type".into(), "A".into()),
                mockito::Matcher::UrlEncoded("name".into(), "lb.example.com".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(list_body(serde_json::json!([{
                "id": "rec-a",
                "name": "lb.example.com",
                "type": "A",
                "content": "203.0.113.10"
            }])))
            .create();

        let client = CloudflareClient::new(server.url(), "token", "zone1");
        let records = client
            .list_address_records(&["lb.example.com.".to_string()], &["A"])
            .unwrap();
        a.assert();
        assert_eq!(records.len(), 1);
    }

    #[test]
    fn retry_after_parses_seconds_and_caps() {
        let mut headers = ureq::http::HeaderMap::new();
        headers.insert("retry-after", "5".parse().unwrap());
        assert_eq!(retry_after_delay(&headers), Some(Duration::from_secs(5)));

        headers.insert("retry-after", "99".parse().unwrap());
        assert_eq!(retry_after_delay(&headers), Some(Duration::from_secs(30)));

        headers.insert("retry-after", "nope".parse().unwrap());
        assert_eq!(retry_after_delay(&headers), None);
    }

    #[test]
    fn retries_rate_limit_then_succeeds() {
        let mut server = mockito::Server::new();
        let limited = server
            .mock("GET", "/zones/zone1/dns_records")
            .match_query(mockito::Matcher::UrlEncoded("type".into(), "A".into()))
            .with_status(429)
            .with_header("retry-after", "0")
            .with_body("rate limited")
            .expect(1)
            .create();
        let ok = server
            .mock("GET", "/zones/zone1/dns_records")
            .match_query(mockito::Matcher::UrlEncoded("type".into(), "A".into()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(list_body(serde_json::json!([])))
            .create();

        let client = CloudflareClient::new(server.url(), "token", "zone1");
        let records = client.list_address_records(&[], &["A"]).unwrap();
        limited.assert();
        ok.assert();
        assert!(records.is_empty());
    }

    #[test]
    fn reads_error_body_from_http_error_status() {
        let mut server = mockito::Server::new();
        let _mock = server
            .mock("GET", "/zones/zone1/dns_records")
            .match_query(mockito::Matcher::UrlEncoded("type".into(), "A".into()))
            .with_status(401)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "success": false,
                    "errors": [{ "message": "Authentication error" }],
                    "result": null
                })
                .to_string(),
            )
            .create();

        let client = CloudflareClient::new(server.url(), "token", "zone1");
        let err = client.list_type("A", None).unwrap_err();
        assert!(err.to_string().contains("Authentication error"));
    }
}
