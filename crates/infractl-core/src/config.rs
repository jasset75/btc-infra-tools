use serde::Deserialize;
use std::collections::HashMap;

pub const DEFAULT_CONFIG_FILE: &str = "belter.toml";

pub fn default_config_template() -> &'static str {
    r#"version = 1
environment = "default"

[service.bitcoind]
manager = "launchd"
unit = "${BITCOIND_LAUNCHD_UNIT}"
tags = ["bitcoin", "core"]

[[check]]
id = "example_http_health"
type = "http"
url = "http://${MEMPOOL_HOST}:${MEMPOOL_PORT}/api/v1/backend-info"
expect = "status == 200"
"#
}

#[derive(Debug, Deserialize)]
pub struct BelterConfig {
    pub service: Option<HashMap<String, ServiceConfig>>,
}

#[derive(Debug, Deserialize)]
pub struct ServiceConfig {
    pub manager: String,
    pub unit: Option<String>,
}
