pub const DEFAULT_CONFIG_FILE: &str = "belter.toml";

pub fn default_config_template() -> &'static str {
    r#"version = 1
environment = "default"

[service.example]
manager = "launchd"
unit = "system/com.example.service"
tags = ["example"]

[[check]]
id = "example_http_health"
type = "http"
url = "http://${MEMPOOL_HOST}:${MEMPOOL_PORT}/api/v1/backend-info"
expect = "status == 200"
"#
}
