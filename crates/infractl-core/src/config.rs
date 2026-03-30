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

[service.podman_runtime]
manager = "podman_machine"
machine = "${PODMAN_MACHINE_NAME}"
tags = ["runtime", "podman"]

[service.stratum]
manager = "launchd"
unit = "${STRATUM_LAUNCHD_UNIT}"
depends_on = ["bitcoind"]
tags = ["mining", "stratum"]

[service.mempool]
manager = "podman_compose"
compose_file = "${MEMPOOL_COMPOSE_FILE}"
compose_override = "${MEMPOOL_COMPOSE_OVERRIDE}"
project = "${MEMPOOL_PROJECT}"
env_file = "${MEMPOOL_ENV_FILE}"
depends_on = ["bitcoind", "podman_runtime"]
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
    pub env_file: Option<String>,
    pub machine: Option<String>,
    pub depends_on: Option<Vec<String>>,
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

    #[test]
    fn default_template_includes_podman_runtime_and_dependencies() {
        let config: BelterConfig =
            toml::from_str(super::default_config_template()).expect("template should parse");

        let podman_runtime = config
            .service_by_name("podman_runtime")
            .expect("template should include podman_runtime service");
        let mempool = config
            .service_by_name("mempool")
            .expect("template should include mempool service");
        let stratum = config
            .service_by_name("stratum")
            .expect("template should include stratum service");

        assert_eq!(podman_runtime.manager, "podman_machine");
        assert_eq!(
            podman_runtime.machine.as_deref(),
            Some("${PODMAN_MACHINE_NAME}")
        );
        assert_eq!(
            stratum.depends_on.as_deref(),
            Some(&["bitcoind".to_string()][..])
        );
        assert_eq!(
            mempool.depends_on.as_deref(),
            Some(&["bitcoind".to_string(), "podman_runtime".to_string()][..])
        );
    }
}
