use crate::output::output_envelope;
use anyhow::{Context, Result, bail};
use infractl_core::time::Clock;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::io::{Read, Write};
use std::net::TcpStream;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PoolInfoResponse {
    #[serde(default)]
    user_agents: Vec<PoolUserAgent>,
    #[serde(default)]
    high_scores: Vec<PoolHighScore>,
    uptime: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PoolUserAgent {
    user_agent: String,
    total_hash_rate: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PoolHighScore {
    updated_at: Option<String>,
    best_difficulty: f64,
    best_difficulty_user_agent: Option<String>,
}

#[derive(Debug, Serialize, PartialEq)]
struct PoolHealthData {
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    best_share: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    best_share_human: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    miner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hashrate_ths: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hashrate_human: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    uptime_since: Option<String>,
}

pub(crate) struct PoolHealthRequest<'a> {
    pub(crate) command_label: &'a str,
    pub(crate) target: Option<&'a str>,
    pub(crate) port: u16,
    pub(crate) explicit_url: Option<&'a str>,
}

pub(crate) fn emit_pool_health<W: Write>(
    clock: &dyn Clock,
    stdout: &mut W,
    json_output: bool,
    dry_run: bool,
    request: PoolHealthRequest<'_>,
) -> Result<()> {
    let url = pool_info_url(request.target, request.port, request.explicit_url);

    if dry_run {
        let out = output_envelope(
            clock,
            request.command_label,
            "ok",
            &format!("would query public-pool health url={url}"),
            true,
            json!({ "url": url, "simulated": true }),
            Vec::new(),
        );
        if json_output {
            writeln!(stdout, "{}", serde_json::to_string_pretty(&out)?)?;
        } else {
            writeln!(stdout, "[{}] {}: {}", out.ts, out.command, out.message)?;
        }
        return Ok(());
    }

    let info = fetch_pool_info(&url)?;
    let data = summarize_pool_info(&url, info);
    let message = render_pool_health_text(&data);
    let out = output_envelope(
        clock,
        request.command_label,
        "ok",
        &message,
        false,
        serde_json::to_value(&data).context("failed to serialize pool health data")?,
        Vec::new(),
    );

    if json_output {
        writeln!(stdout, "{}", serde_json::to_string_pretty(&out)?)?;
    } else {
        writeln!(stdout, "[{}] {}: {}", out.ts, out.command, out.message)?;
    }

    Ok(())
}

fn pool_info_url(target: Option<&str>, port: u16, explicit_url: Option<&str>) -> String {
    if let Some(url) = explicit_url {
        return url.to_string();
    }

    let host = target.unwrap_or("127.0.0.1");
    format!("http://{host}:{port}/api/info")
}

fn summarize_pool_info(url: &str, info: PoolInfoResponse) -> PoolHealthData {
    let best = info.high_scores.first();
    let agent = info.user_agents.first();
    let best_share = best.map(|entry| entry.best_difficulty);
    let hashrate_ths = agent
        .and_then(|entry| entry.total_hash_rate)
        .map(|value| value / 1e12);

    PoolHealthData {
        url: url.to_string(),
        best_share,
        best_share_human: best_share.map(human_metric),
        miner: best
            .and_then(|entry| entry.best_difficulty_user_agent.clone())
            .or_else(|| agent.map(|entry| entry.user_agent.clone())),
        updated_at: best.and_then(|entry| entry.updated_at.clone()),
        hashrate_ths,
        hashrate_human: hashrate_ths.map(|value| format!("{value:.2} TH/s")),
        uptime_since: info.uptime,
    }
}

fn render_pool_health_text(data: &PoolHealthData) -> String {
    let best_share = data.best_share_human.as_deref().unwrap_or("n/a");
    let miner = data.miner.as_deref().unwrap_or("n/a");
    let hashrate = data.hashrate_human.as_deref().unwrap_or("n/a");
    let updated_at = data.updated_at.as_deref().unwrap_or("n/a");
    let uptime_since = data.uptime_since.as_deref().unwrap_or("n/a");

    format!(
        "best_share={best_share} miner={miner} hashrate={hashrate} updated={updated_at} uptime_since={uptime_since}"
    )
}

fn human_metric(value: f64) -> String {
    let (scaled, suffix) = if value >= 1e12 {
        (value / 1e12, "T")
    } else if value >= 1e9 {
        (value / 1e9, "G")
    } else if value >= 1e6 {
        (value / 1e6, "M")
    } else if value >= 1e3 {
        (value / 1e3, "K")
    } else {
        (value, "")
    };

    if suffix.is_empty() {
        format!("{scaled:.0}")
    } else {
        format!("{scaled:.2}{suffix}")
    }
}

fn fetch_pool_info(url: &str) -> Result<PoolInfoResponse> {
    let endpoint = parse_http_url(url)?;
    let mut stream = TcpStream::connect((&endpoint.host[..], endpoint.port))
        .with_context(|| format!("failed to connect to {}", endpoint.authority()))?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(3)))
        .context("failed to set read timeout")?;
    stream
        .set_write_timeout(Some(std::time::Duration::from_secs(3)))
        .context("failed to set write timeout")?;

    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept: application/json\r\n\r\n",
        endpoint.path,
        endpoint.authority()
    );
    stream
        .write_all(request.as_bytes())
        .context("failed to write HTTP request")?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .context("failed to read HTTP response")?;

    let (_, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| anyhow::anyhow!("invalid HTTP response from {}", endpoint.authority()))?;
    let status_line = response.lines().next().unwrap_or_default();
    if !status_line.contains(" 200 ") {
        bail!(
            "unexpected HTTP status from {}: {}",
            endpoint.authority(),
            status_line
        );
    }

    serde_json::from_str(body).with_context(|| format!("failed to parse JSON from {url}"))
}

struct HttpEndpoint {
    host: String,
    port: u16,
    path: String,
}

impl HttpEndpoint {
    fn authority(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

fn parse_http_url(url: &str) -> Result<HttpEndpoint> {
    let without_scheme = url
        .strip_prefix("http://")
        .ok_or_else(|| anyhow::anyhow!("only http:// URLs are currently supported"))?;
    let (authority, path_part) = match without_scheme.split_once('/') {
        Some((authority, path)) => (authority, format!("/{path}")),
        None => (without_scheme, "/".to_string()),
    };
    let (host, port) = match authority.split_once(':') {
        Some((host, port)) => (
            host.to_string(),
            port.parse::<u16>()
                .with_context(|| format!("invalid port in URL `{url}`"))?,
        ),
        None => (authority.to_string(), 80),
    };

    if host.is_empty() {
        bail!("missing host in URL `{url}`");
    }

    Ok(HttpEndpoint {
        host,
        port,
        path: path_part,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_metric_formats_large_values() {
        assert_eq!(human_metric(123_606_074.140_3), "123.61M");
        assert_eq!(human_metric(3_250_000_000.0), "3.25G");
        assert_eq!(human_metric(950.0), "950");
    }

    #[test]
    fn summarize_pool_info_prefers_high_score_for_best_share() {
        let data = summarize_pool_info(
            "http://127.0.0.1:3334/api/info",
            PoolInfoResponse {
                user_agents: vec![PoolUserAgent {
                    user_agent: "bitaxe".to_string(),
                    total_hash_rate: Some(1_385_325_271_934.692_9),
                }],
                high_scores: vec![PoolHighScore {
                    updated_at: Some("2026-03-08 17:18:56".to_string()),
                    best_difficulty: 123_606_074.14031632,
                    best_difficulty_user_agent: Some("bitaxe".to_string()),
                }],
                uptime: Some("2026-03-13T17:33:04.569Z".to_string()),
            },
        );

        assert_eq!(data.best_share, Some(123_606_074.14031632));
        assert_eq!(data.best_share_human.as_deref(), Some("123.61M"));
        assert_eq!(data.hashrate_human.as_deref(), Some("1.39 TH/s"));
    }

    #[test]
    fn pool_info_url_uses_target_and_default_port() {
        assert_eq!(
            pool_info_url(Some("192.0.2.10"), 3334, None),
            "http://192.0.2.10:3334/api/info"
        );
    }

    #[test]
    fn pool_info_url_prefers_explicit_url() {
        assert_eq!(
            pool_info_url(
                Some("ignored"),
                9999,
                Some("http://192.0.2.10:3334/api/info")
            ),
            "http://192.0.2.10:3334/api/info"
        );
    }
}
