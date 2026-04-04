use crate::traits::*;
use async_trait::async_trait;
use hickory_resolver::TokioAsyncResolver;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use serde::Deserialize;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use tokio::sync::OnceCell;

static HTTP_CLIENT: OnceCell<Result<reqwest::Client, anyhow::Error>> = OnceCell::const_new();

async fn get_http_client() -> &'static reqwest::Client {
    HTTP_CLIENT
        .get_or_init(|| async {
            let resolver = TokioAsyncResolver::tokio_from_system_conf()
                .map_err(|e| anyhow::anyhow!("Failed to create DNS resolver: {}", e))?;

            reqwest::Client::builder()
                .user_agent("kn-code/0.1.0")
                .timeout(std::time::Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    if attempt.previous().len() > 5 {
                        return attempt.error("too many redirects");
                    }
                    let url = attempt.url();
                    if let Err(e) = validate_url(url.as_str()) {
                        return attempt.error(e);
                    }
                    attempt.follow()
                }))
                .dns_resolver(Arc::new(SafeDnsResolver { resolver }))
                .build()
                .map_err(|e| anyhow::anyhow!("Failed to build HTTP client: {}", e))
        })
        .await
        .as_ref()
        .expect("HTTP client initialization failed — this is a fatal configuration error")
}

struct SafeDnsResolver {
    resolver: TokioAsyncResolver,
}

impl Resolve for SafeDnsResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let resolver = self.resolver.clone();
        Box::pin(async move {
            let lookup = resolver.lookup_ip(name.as_str()).await.map_err(
                |e| -> Box<dyn std::error::Error + Send + Sync> {
                    Box::new(std::io::Error::other(e.to_string()))
                },
            )?;

            let addrs: Vec<SocketAddr> = lookup
                .into_iter()
                .filter(|ip| !is_private_ip(&ip.to_string()))
                .map(|ip| SocketAddr::new(ip, 0))
                .collect();

            if addrs.is_empty() {
                return Err(Box::new(std::io::Error::other(
                    "All resolved addresses are in private/internal ranges",
                ))
                    as Box<dyn std::error::Error + Send + Sync>);
            }

            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

#[derive(Debug)]
pub struct WebFetchTool;

impl Default for WebFetchTool {
    fn default() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize)]
struct WebFetchInput {
    url: String,
    #[serde(default)]
    max_length: Option<usize>,
}

fn is_ipv4_in_range(ip: &Ipv4Addr, base: &Ipv4Addr, prefix_len: u8) -> bool {
    let ip_bits = u32::from_be_bytes(ip.octets());
    let base_bits = u32::from_be_bytes(base.octets());
    if prefix_len == 0 {
        return true;
    }
    let mask = !((1u32 << (32 - prefix_len)) - 1);
    (ip_bits & mask) == (base_bits & mask)
}

fn is_private_ip(host: &str) -> bool {
    if let Ok(ip) = host.parse::<IpAddr>() {
        match ip {
            IpAddr::V4(ipv4) => {
                if ipv4.is_loopback() {
                    return true;
                }
                if is_ipv4_in_range(&ipv4, &Ipv4Addr::new(10, 0, 0, 0), 8) {
                    return true;
                }
                if is_ipv4_in_range(&ipv4, &Ipv4Addr::new(172, 16, 0, 0), 12) {
                    return true;
                }
                if is_ipv4_in_range(&ipv4, &Ipv4Addr::new(192, 168, 0, 0), 16) {
                    return true;
                }
                if is_ipv4_in_range(&ipv4, &Ipv4Addr::new(169, 254, 0, 0), 16) {
                    return true;
                }
                if is_ipv4_in_range(&ipv4, &Ipv4Addr::new(100, 64, 0, 0), 10) {
                    return true;
                }
                if is_ipv4_in_range(&ipv4, &Ipv4Addr::new(127, 0, 0, 0), 8) {
                    return true;
                }
            }
            IpAddr::V6(ipv6) => {
                if ipv6.is_loopback() {
                    return true;
                }
                let segments = ipv6.segments();
                if (segments[0] & 0xfe00) == 0xfc00 {
                    return true;
                }
                if (segments[0] & 0xffc0) == 0xfe80 {
                    return true;
                }
                if segments[0] == 0xfd00 {
                    return true;
                }
            }
        }
    }
    false
}

fn validate_url(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("Invalid URL: {}", e))?;

    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(format!(
            "Unsupported scheme: {}. Only http and https are allowed.",
            scheme
        ));
    }

    let host = parsed.host_str().ok_or("URL has no host")?;
    if is_private_ip(host) {
        return Err(format!(
            "Access to internal addresses is not allowed: {}",
            host
        ));
    }

    let lower = host.to_lowercase();
    if lower == "localhost"
        || lower == "127.0.0.1"
        || lower == "0.0.0.0"
        || lower == "169.254.169.254"
        || lower.ends_with(".internal")
        || lower.ends_with(".local")
        || lower.ends_with(".lan")
        || lower.ends_with(".consul")
        || lower.ends_with(".vault")
    {
        return Err(format!(
            "Access to internal addresses is not allowed: {}",
            host
        ));
    }

    Ok(())
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "WebFetch"
    }
    fn description(&self) -> &str {
        "Fetch content from a URL"
    }
    fn prompt(&self) -> &str {
        "Use this to fetch web content. Converts HTML to markdown."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "URL to fetch" },
                "max_length": { "type": "integer", "description": "Max characters to return" }
            },
            "required": ["url"]
        })
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn call(
        &self,
        input: serde_json::Value,
        _context: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let parsed: WebFetchInput =
            serde_json::from_value(input.clone()).map_err(|e| ToolError::ValidationFailed {
                message: e.to_string(),
            })?;

        validate_url(&parsed.url).map_err(|e| ToolError::ValidationFailed {
            message: e.to_string(),
        })?;

        let max_length = parsed.max_length.unwrap_or(1_000_000);

        let client = get_http_client().await;

        let response = client
            .get(&parsed.url)
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to fetch URL: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            return Ok(ToolResult {
                content: ToolContent::Text(format!(
                    "HTTP {}: Failed to fetch {}",
                    status, parsed.url
                )),
                new_messages: Vec::new(),
                persisted: false,
                persisted_path: None,
                structured_content: None,
            });
        }

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let body = response
            .text()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read response: {}", e)))?;

        let text = if content_type.contains("html") {
            html2md::parse_html(&body)
        } else {
            body
        };

        let text_len = text.len();
        let truncated = if text_len > max_length {
            let end = text
                .char_indices()
                .take_while(|(idx, _)| *idx <= max_length)
                .last()
                .map_or(0, |(idx, c)| idx + c.len_utf8());
            format!("{}... (truncated, {} total chars)", &text[..end], text_len)
        } else {
            text
        };

        Ok(ToolResult {
            content: ToolContent::Text(truncated),
            new_messages: Vec::new(),
            persisted: false,
            persisted_path: None,
            structured_content: Some(serde_json::json!({
                "url": parsed.url,
                "content_type": content_type,
                "content_length": text_len,
                "truncated": text_len > max_length,
            })),
        })
    }
}
