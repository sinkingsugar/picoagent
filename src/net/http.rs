//! HTTP client helpers using the same API as bme680-monitor.
//!
//! Uses EspHttpConnection directly with initiate_request/initiate_response.
//! Proven to work on Freenove ESP32-S3 WROOM Lite.

use anyhow::{bail, Result};
use esp_idf_svc::http::client::{Configuration, EspHttpConnection};
use esp_idf_svc::io::Write;
use log::debug;

/// Create an HTTP connection with TLS configured.
fn make_connection(timeout_secs: u64) -> Result<EspHttpConnection> {
    let config = Configuration {
        use_global_ca_store: true,
        crt_bundle_attach: Some(esp_idf_svc::sys::esp_crt_bundle_attach),
        timeout: Some(std::time::Duration::from_secs(timeout_secs)),
        ..Default::default()
    };
    Ok(EspHttpConnection::new(&config)?)
}

/// Perform an HTTP GET and return the response body as a String.
pub fn get(url: &str, timeout_secs: u64) -> Result<(u16, String)> {
    let mut client = make_connection(timeout_secs)?;

    client.initiate_request(esp_idf_svc::http::Method::Get, url, &[])?;
    client.initiate_response()?;

    let status = client.status();
    let body = read_response_body(&mut client)?;

    debug!("GET -> {} ({} bytes)", status, body.len());
    Ok((status, body))
}

/// Perform an HTTP POST with a JSON body and return the response.
pub fn post_json(
    url: &str,
    body: &str,
    extra_headers: &[(&str, &str)],
    timeout_secs: u64,
) -> Result<(u16, String)> {
    let mut client = make_connection(timeout_secs)?;

    let content_length = body.len().to_string();

    // Build headers: content-type + content-length + extras
    let mut headers: Vec<(&str, &str)> = vec![
        ("Content-Type", "application/json"),
        ("Content-Length", &content_length),
    ];
    headers.extend_from_slice(extra_headers);

    client.initiate_request(esp_idf_svc::http::Method::Post, url, &headers)?;
    client.write_all(body.as_bytes())?;
    client.flush()?;
    client.initiate_response()?;

    let status = client.status();
    let body = read_response_body(&mut client)?;

    debug!("POST -> {} ({} bytes)", status, body.len());
    Ok((status, body))
}

/// Maximum response body size (128 KB) — protects against OOM on ESP32.
const MAX_RESPONSE_BYTES: usize = 128 * 1024;

/// Read the full response body into a String.
fn read_response_body(client: &mut EspHttpConnection) -> Result<String> {
    let mut buf = [0u8; 2048];
    let mut body = Vec::new();

    loop {
        match esp_idf_svc::io::Read::read(client, &mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if body.len().saturating_add(n) > MAX_RESPONSE_BYTES {
                    bail!("response too large (>{} KB)", MAX_RESPONSE_BYTES / 1024);
                }
                body.extend_from_slice(&buf[..n]);
            }
            Err(e) => bail!("failed to read response: {e}"),
        }
    }

    String::from_utf8(body).map_err(|e| anyhow::anyhow!("response not UTF-8: {e}"))
}
