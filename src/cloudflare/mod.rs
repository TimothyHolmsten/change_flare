use reqwest::header::{HeaderMap, HeaderValue};
use std::{env, net::IpAddr};
use ureq::serde_json;

use serde::Deserialize;

use crate::core::{ApiTrait, Record};
const CLOUDFLARE_POLL_RATE: &str = "CLOUDFLARE_POLL_RATE";
const CLOUDFLARE_API_KEY: &str = "CLOUDFLARE_API_KEY";
const CLOUDFLARE_ZONE_ID: &str = "CLOUDFLARE_ZONE_ID";

#[derive(Default)]
pub struct CloudFlareApi {
    config: CloudFlareConfig,
    records: Vec<CloudFlareRecord>,
}

#[allow(refining_impl_trait)]
impl ApiTrait for CloudFlareApi {
    type RecordType = CloudFlareRecord;

    fn new(poll_rate: usize, api_key: String) -> Self {
        Self {
            config: CloudFlareConfig::new(poll_rate, api_key),
            records: Vec::new(),
        }
    }

    fn get_records(&mut self) -> &Vec<CloudFlareRecord> {
        let client = reqwest::blocking::Client::new();
        let mut headers = HeaderMap::new();

        let auth_header = match HeaderValue::from_str(&format!("Bearer {}", self.config.api_key)) {
            Ok(header) => header,
            Err(e) => {
                eprintln!("Invalid API key format: {}", e);
                return &self.records;
            }
        };
        headers.insert("Authorization", auth_header);

        let response = match client
            .get(&format!(
                "https://api.cloudflare.com/client/v4/zones/{}/dns_records",
                self.config.zone_id
            ))
            .headers(headers)
            .send()
        {
            Ok(resp) => resp,
            Err(e) => {
                eprintln!("Failed to send request: {}", e);
                return &self.records;
            }
        };

        let response_text = match response.text() {
            Ok(text) => text,
            Err(e) => {
                eprintln!("Failed to get response text: {}", e);
                return &self.records;
            }
        };

        let response: CloudflareResponse = match serde_json::from_str(&response_text) {
            Ok(resp) => resp,
            Err(e) => {
                eprintln!(
                    "Failed to parse response: {}\nResponse text: {}",
                    e, response_text
                );
                return &self.records;
            }
        };

        if !response.success {
            eprintln!("Cloudflare API request failed");
            return &self.records;
        }

        self.records = response
            .result
            .into_iter()
            .filter_map(|r| {
                let content = match r.content.parse() {
                    Ok(ip) => ip,
                    Err(e) => {
                        eprintln!("Invalid IP address for record {}: {}", r.name, e);
                        return None;
                    }
                };

                Some(CloudFlareRecord {
                    content,
                    name: r.name,
                    record_type: r.r#type,
                    ttl: r.ttl,
                    proxied: r.proxied,
                    zone_id: r.zone_id,
                    record_id: Some(r.id),
                })
            })
            .collect();

        &self.records
    }
    fn update_record(&mut self, record: &CloudFlareRecord) -> CloudFlareRecord {
        let client = reqwest::blocking::Client::new();
        let mut headers = HeaderMap::new();

        headers.insert(
            "Authorization",
            HeaderValue::from_str(&format!("Bearer {}", self.config.api_key))
                .map_err(|e| eprintln!("Invalid API key format: {}", e))
                .unwrap_or_else(|e| {
                    eprintln!("Invalid API key format");
                    HeaderValue::from_static("")
                }),
        );
        headers.insert("Content-Type", HeaderValue::from_static("application/json"));

        let url = format!(
            "https://api.cloudflare.com/client/v4/zones/{}/dns_records/{}",
            self.config.zone_id,
            record.get_id().unwrap_or_else(|| {
                eprintln!("Record ID not found");
                String::new()
            })
        );

        let payload = serde_json::json!({
            "content": record.content.to_string(),
            "name": record.name,
            "proxied": record.proxied,
            "type": record.record_type,
            "ttl": record.ttl,
        });

        match client
            .put(&url)
            .headers(headers)
            .json(&payload)
            .send()
            .and_then(|r| r.error_for_status())
        {
            Ok(_) => record.clone(),
            Err(e) => {
                eprintln!("Failed to update record: {}", e);
                record.clone()
            }
        }
    }

    fn get_poll_rate(&self) -> usize {
        self.config.poll_rate
    }
}

struct CloudFlareConfig {
    poll_rate: usize,
    api_key: String,
    zone_id: String,
}

impl Default for CloudFlareConfig {
    fn default() -> Self {
        if let Err(e) = dotenvy::dotenv() {
            if e.not_found() {
                eprintln!(".env file was not found, please create and configure .env");
            } else {
                eprintln!(".env file was found but error has occurred: {}", e);
            }
        }

        let poll_rate = env::var(CLOUDFLARE_POLL_RATE)
            .ok()
            .and_then(|val| val.parse::<usize>().ok())
            .unwrap_or(300);

        let api_key = env::var(CLOUDFLARE_API_KEY).unwrap_or_else(|_| {
            eprintln!(
                "Cloudflare API key was not found. Please configure {}",
                CLOUDFLARE_API_KEY
            );
            String::new()
        });

        let zone_id = env::var(CLOUDFLARE_ZONE_ID).unwrap_or_else(|_| {
            eprintln!(
                "Cloudflare zone ID was not found. Please configure {}",
                CLOUDFLARE_ZONE_ID
            );
            String::new()
        });

        Self {
            poll_rate,
            api_key,
            zone_id,
        }
    }
}

impl CloudFlareConfig {
    fn new(mut poll_rate: usize, mut api_key: String) -> Self {
        poll_rate = poll_rate.max(60);
        println!("Cloudflare polling rate set to {} seconds", poll_rate);

        api_key = if api_key.is_empty() {
            println!("CloudFlare API key was empty, trying to configure using .env file");
            Self::default().api_key
        } else {
            api_key
        };

        let zone_id = Self::default().zone_id;

        return Self {
            poll_rate: poll_rate,
            api_key: api_key,
            zone_id: zone_id,
        };
    }
}

#[derive(Clone)]
pub struct CloudFlareRecord {
    content: IpAddr, // IP address
    name: String,
    record_type: String, // A, AAAA, CNAME, etc.
    ttl: u32,
    proxied: bool,
    zone_id: String,           // Remove this?
    record_id: Option<String>, // Only used if updating an existing record
}

impl Record<CloudFlareApi> for CloudFlareRecord {
    fn get_id(&self) -> Option<String> {
        self.record_id.clone()
    }

    fn get_name(&self) -> String {
        self.name.clone()
    }

    fn get_content(&self) -> IpAddr {
        self.content.clone()
    }

    fn update_content(&self, new_content: IpAddr) -> Self {
        Self {
            content: new_content,
            ..self.clone()
        }
    }
}

#[derive(Deserialize)]
struct CloudflareResponse {
    success: bool,
    result: Vec<CloudflareResult>,
}

#[derive(Deserialize)]
struct CloudflareResult {
    id: String,
    name: String,
    content: String,
    r#type: String,
    ttl: u32,
    proxied: bool,
    zone_id: String,
}

#[cfg(test)]
mod tests {
    use mockito::Mock;

    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    fn setup_mock_api() -> (CloudFlareApi, String) {
        let api_key = "test_api_key".to_string();
        let zone_id = "test_zone_id".to_string();

        let mut api = CloudFlareApi::new(60, api_key.clone());
        api.config.zone_id = zone_id.clone();

        (api, zone_id)
    }

    fn create_mock_record() -> CloudFlareRecord {
        CloudFlareRecord {
            content: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            name: "test.example.com".to_string(),
            record_type: "A".to_string(),
            ttl: 1,
            proxied: false,
            zone_id: "test_zone_id".to_string(),
            record_id: Some("record123".to_string()),
        }
    }

    #[test]
    fn test_cloudflare_config_new() {
        let api_key = "test_key".to_string();
        let config = CloudFlareConfig::new(30, api_key.clone());

        assert_eq!(config.poll_rate, 60); // Should be minimum 60
        assert_eq!(config.api_key, api_key);
    }

    #[test]
    fn test_record_update_content() {
        let record = create_mock_record();
        let new_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));

        let updated_record = record.update_content(new_ip);

        assert_eq!(updated_record.content, new_ip);
        assert_eq!(updated_record.name, record.name);
        assert_eq!(updated_record.record_type, record.record_type);
    }


    #[test]
    fn test_update_record() {
        let (mut api, zone_id) = setup_mock_api();
        let record = create_mock_record();

        let mock_response = ureq::json!({
            "success": true,
            "result": {
                "id": "record123",
                "name": "test.example.com",
                "content": "127.0.0.1",
                "type": "A",
                "ttl": 1,
                "proxied": false,
                "zone_id": zone_id
            }
        });

        let mut server = mockito::Server::new();
        let _m = server
            .mock("PUT", "/client/v4/zones/test_zone_id/dns_records/record123")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(mock_response.to_string())
            .create();

        let updated_record = api.update_record(&record);

        assert_eq!(updated_record.name, record.name);
        assert_eq!(updated_record.content, record.content);
    }
}
