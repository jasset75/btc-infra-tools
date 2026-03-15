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

[service.stratum]
manager = "launchd"
unit = "${STRATUM_LAUNCHD_UNIT}"
tags = ["mining", "stratum"]

[service.mempool]
manager = "podman_compose"
compose_file = "${MEMPOOL_COMPOSE_FILE}"
compose_override = "${MEMPOOL_COMPOSE_OVERRIDE}"
project = "${MEMPOOL_PROJECT}"
tags = ["explorer"]

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

impl BelterConfig {
    pub fn service_by_name(&self, name: &str) -> Option<&ServiceConfig> {
        self.service.as_ref()?.get(name)
    }

    pub fn stratum_service(&self) -> Option<&ServiceConfig> {
        self.service_by_name("stratum")
    }
}

#[derive(Debug, Deserialize)]
pub struct ServiceConfig {
    pub manager: String,
    pub unit: Option<String>,
    pub compose_file: Option<String>,
    pub compose_override: Option<String>,
    pub project: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::BelterConfig;

    #[test]
    fn parses_stratum_service_contract_from_toml() {
        let raw = r#"
[service.stratum]
manager = "launchd"
unit = "system/io.btc.public-pool"
"#;

        let config: BelterConfig = toml::from_str(raw).expect("config should parse");
        let stratum = config
            .stratum_service()
            .expect("stratum service should exist");

        assert_eq!(stratum.manager, "launchd");
        assert_eq!(stratum.unit.as_deref(), Some("system/io.btc.public-pool"));
    }

    #[test]
    fn stratum_service_returns_none_when_missing() {
        let raw = r#"
[service.bitcoind]
manager = "launchd"
unit = "system/com.bitcoind.node"
"#;

        let config: BelterConfig = toml::from_str(raw).expect("config should parse");
        assert!(config.stratum_service().is_none());
    }

    #[test]
    fn default_template_includes_stratum_service() {
        let config: BelterConfig =
            toml::from_str(super::default_config_template()).expect("template should parse");
        let stratum = config
            .stratum_service()
            .expect("template should include stratum service");

        assert_eq!(stratum.manager, "launchd");
        assert_eq!(stratum.unit.as_deref(), Some("${STRATUM_LAUNCHD_UNIT}"));
    }
}
