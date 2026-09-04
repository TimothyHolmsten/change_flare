use std::net::IpAddr;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use ureq::Agent;

use crate::config::name_matches;
use crate::error::Error;

const USER_AGENT: &str = concat!("change_flare/", env!("CARGO_PKG_VERSION"));
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);
const PAGE_SIZE: u32 = 100;

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
            .build()
            .into();

        Self {
            agent,
            api_base: api_base.into().trim_end_matches('/').to_string(),
            authorization: format!("Bearer {}", token.into()),
            zone_id: zone_id.into(),
        }
    }

    /// List A/AAAA records, optionally restricted to `record_names`.
    pub fn list_address_records(&self, record_names: &[String]) -> Result<Vec<DnsRecord>, Error> {
        let mut records = Vec::new();
        for record_type in ["A", "AAAA"] {
            records.extend(self.list_type(record_type)?);
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

    fn list_type(&self, record_type: &str) -> Result<Vec<DnsRecord>, Error> {
        let mut page = 1u32;
        let mut records = Vec::new();

        loop {
            let url = format!(
                "{}/zones/{}/dns_records?type={record_type}&per_page={PAGE_SIZE}&page={page}",
                self.api_base, self.zone_id
            );
            let mut response = self
                .agent
                .get(&url)
                .header("Authorization", &self.authorization)
                .header("User-Agent", USER_AGENT)
                .call()?;

            let parsed: ApiResponse<Vec<RawRecord>> = response.body_mut().read_json()?;
            if !parsed.success {
                return Err(api_error(parsed.errors));
            }

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
        let body = PatchBody {
            content: &content.to_string(),
        };
        let mut response = self
            .agent
            .patch(&url)
            .header("Authorization", &self.authorization)
            .header("User-Agent", USER_AGENT)
            .header("Content-Type", "application/json")
            .send_json(&body)?;

        let parsed: ApiResponse<RawRecord> = response.body_mut().read_json()?;
        if !parsed.success {
            return Err(api_error(parsed.errors));
        }
        Ok(())
    }
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
        let records = client.list_address_records(&["lb".to_string()]).unwrap();

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
        let err = client.list_type("A").unwrap_err();
        assert!(err.to_string().contains("Authentication error"));
    }
}
