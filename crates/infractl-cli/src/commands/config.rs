use anyhow::{Context, Result, bail};
use infractl_core::config::{BelterConfig, default_config_template};
use infractl_core::env::{EnvResolver, expand_placeholders};
use std::fs;
use std::path::PathBuf;

const LAUNCHD_MANAGER: &str = "launchd";
const PODMAN_COMPOSE_MANAGER: &str = "podman_compose";
const PODMAN_MACHINE_MANAGER: &str = "podman_machine";
const REQUIRED_SERVICES: [&str; 3] = ["bitcoind", "stratum", "mempool"];

pub(crate) fn init_config_file(path: &PathBuf, force: bool) -> Result<()> {
    if path.exists() && !force {
        bail!(
            "config file already exists at {} (use --force to overwrite)",
            path.display()
        );
    }

    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }

    fs::write(path, default_config_template())
        .with_context(|| format!("failed to write config file {}", path.display()))?;
    Ok(())
}

pub(crate) fn validate_config_file(
    env_resolver: &dyn EnvResolver,
    path: &PathBuf,
    write_missing: bool,
) -> Result<String> {
    let mut raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read config file {}", path.display()))?;
    let mut config: BelterConfig = toml::from_str(&raw)
        .with_context(|| format!("failed to parse TOML from {}", path.display()))?;

    let mut added = Vec::new();
    if write_missing {
        added = append_missing_required_services(path, &mut raw, &config)?;
        if !added.is_empty() {
            config = toml::from_str(&raw)
                .with_context(|| format!("failed to parse TOML from {}", path.display()))?;
        }
    }

    let services = config
        .service
        .as_ref()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "missing [service] section.\n\nExample:\n{}",
                example_service_block("bitcoind")
            )
        })?;
    if services.is_empty() {
        bail!("missing service definitions under [service]");
    }

    for required in REQUIRED_SERVICES {
        if !services.contains_key(required) {
            bail!(
                "missing required service `{required}` in config.\n\nExample:\n{}",
                example_service_block(required)
            );
        }
    }

    for (name, service) in services {
        match service.manager.as_str() {
            LAUNCHD_MANAGER => {
                let unit_tmpl = service
                    .unit
                    .as_deref()
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "service `{name}` is missing `unit`.\n\nExample:\n{}",
                            example_service_block(name)
                        )
                    })?;
                resolve_placeholder_for_field(env_resolver, name, "unit", unit_tmpl)?;
            }
            PODMAN_COMPOSE_MANAGER => {
                let compose_file_tmpl = service.compose_file.as_deref().ok_or_else(|| {
                    anyhow::anyhow!(
                        "service `{name}` is missing `compose_file`.\n\nExample:\n{}",
                        example_service_block(name)
                    )
                })?;
                resolve_placeholder_for_field(
                    env_resolver,
                    name,
                    "compose_file",
                    compose_file_tmpl,
                )?;

                if let Some(value) = service.compose_override.as_deref() {
                    resolve_placeholder_for_field(env_resolver, name, "compose_override", value)?;
                }

                if let Some(value) = service.project.as_deref() {
                    resolve_placeholder_for_field(env_resolver, name, "project", value)?;
                }

                if let Some(value) = service.env_file.as_deref() {
                    resolve_placeholder_for_field(env_resolver, name, "env_file", value)?;
                }
            }
            PODMAN_MACHINE_MANAGER => {
                let machine_tmpl = service.machine.as_deref().ok_or_else(|| {
                    anyhow::anyhow!(
                        "service `{name}` is missing `machine`.\n\nExample:\n{}",
                        example_service_block(name)
                    )
                })?;
                resolve_placeholder_for_field(env_resolver, name, "machine", machine_tmpl)?;
            }
            other => bail!("service `{name}` has unsupported manager `{other}`"),
        }
    }

    if added.is_empty() {
        Ok(format!(
            "configuration is valid (services checked: {})",
            services.len()
        ))
    } else {
        Ok(format!(
            "configuration is valid (services checked: {}, auto-added missing: {})",
            services.len(),
            added.join(", ")
        ))
    }
}

fn append_missing_required_services(
    path: &PathBuf,
    raw: &mut String,
    config: &BelterConfig,
) -> Result<Vec<String>> {
    let existing = config.service.as_ref();
    let missing: Vec<&str> = REQUIRED_SERVICES
        .iter()
        .copied()
        .filter(|name| existing.and_then(|map| map.get(*name)).is_none())
        .collect();

    if missing.is_empty() {
        return Ok(Vec::new());
    }

    if !raw.ends_with('\n') {
        raw.push('\n');
    }
    raw.push('\n');
    for name in &missing {
        raw.push_str(example_service_block(name));
        raw.push('\n');
        raw.push('\n');
    }

    fs::write(path, raw).with_context(|| format!("failed to write config file {}", path.display()))?;
    Ok(missing.into_iter().map(ToOwned::to_owned).collect())
}

fn resolve_placeholder_for_field(
    env_resolver: &dyn EnvResolver,
    service_name: &str,
    field: &str,
    template: &str,
) -> Result<String> {
    match expand_placeholders(template, env_resolver) {
        Ok(value) => Ok(value),
        Err(err) => {
            if let Some(var) = missing_env_var_name(&err.to_string()) {
                bail!(
                    "service `{service_name}` field `{field}` requires missing env var `{var}`. Set it in `.env` or export it in your shell (example: `export {var}=...`). Template: `{template}`"
                );
            }
            bail!(
                "service `{service_name}` failed to resolve `{field}` placeholder(s): {err}"
            );
        }
    }
}

fn missing_env_var_name(message: &str) -> Option<String> {
    let marker = "missing environment variable `";
    let start = message.find(marker)? + marker.len();
    let tail = &message[start..];
    let end = tail.find('`')?;
    Some(tail[..end].to_string())
}

fn example_service_block(name: &str) -> &'static str {
    match name {
        "bitcoind" => {
            r#"[service.bitcoind]
manager = "launchd"
unit = "${BITCOIND_LAUNCHD_UNIT}""#
        }
        "stratum" => {
            r#"[service.stratum]
manager = "launchd"
unit = "${STRATUM_LAUNCHD_UNIT}""#
        }
        "mempool" => {
            r#"[service.mempool]
manager = "podman_compose"
compose_file = "${MEMPOOL_COMPOSE_FILE}"
compose_override = "${MEMPOOL_COMPOSE_OVERRIDE}"
project = "${MEMPOOL_PROJECT}"
env_file = "${MEMPOOL_ENV_FILE}""#
        }
        "podman_runtime" => {
            r#"[service.podman_runtime]
manager = "podman_machine"
machine = "${PODMAN_MACHINE_NAME}""#
        }
        _ => {
            r#"[service.<name>]
manager = "launchd"
unit = "${SERVICE_UNIT}""#
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{init_config_file, validate_config_file};
    use infractl_core::env::FixedEnvResolver;
    use std::collections::HashMap;
    use std::fs;
    use std::io::ErrorKind;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn init_config_writes_stratum_service_template() {
        let dir = unique_fixture_dir();
        fs::create_dir_all(&dir).expect("fixture dir should be created");
        let path = dir.join("belter.toml");

        init_config_file(&path, false).expect("config init should succeed");

        let content = fs::read_to_string(&path).expect("config file should be readable");
        assert!(content.contains("[service.stratum]"));
        assert!(content.contains("unit = \"${STRATUM_LAUNCHD_UNIT}\""));

        remove_fixture_dir(&dir);
    }

    #[test]
    fn validate_config_file_accepts_required_services_with_resolved_placeholders() {
        let dir = unique_fixture_dir();
        fs::create_dir_all(&dir).expect("fixture dir should be created");
        let path = dir.join("belter.toml");
        fs::write(
            &path,
            r#"
[service.bitcoind]
manager = "launchd"
unit = "${BITCOIND_LAUNCHD_UNIT}"

[service.stratum]
manager = "launchd"
unit = "${STRATUM_LAUNCHD_UNIT}"

[service.mempool]
manager = "podman_compose"
compose_file = "${MEMPOOL_COMPOSE_FILE}"
compose_override = "${MEMPOOL_COMPOSE_OVERRIDE}"
project = "${MEMPOOL_PROJECT}"
env_file = "${MEMPOOL_ENV_FILE}"
"#,
        )
        .expect("config should be written");

        let env = FixedEnvResolver::new(HashMap::from([
            (
                "BITCOIND_LAUNCHD_UNIT".to_string(),
                "system/com.bitcoind.node".to_string(),
            ),
            (
                "STRATUM_LAUNCHD_UNIT".to_string(),
                "system/io.btc.public-pool".to_string(),
            ),
            (
                "MEMPOOL_COMPOSE_FILE".to_string(),
                "/tmp/mempool.yml".to_string(),
            ),
            (
                "MEMPOOL_COMPOSE_OVERRIDE".to_string(),
                "/tmp/mempool.override.yml".to_string(),
            ),
            ("MEMPOOL_PROJECT".to_string(), "mempool".to_string()),
            ("MEMPOOL_ENV_FILE".to_string(), "/tmp/mempool.env".to_string()),
        ]));

        let message = validate_config_file(&env, &path, false).expect("config should validate");
        assert!(message.contains("configuration is valid"));

        remove_fixture_dir(&dir);
    }

    #[test]
    fn validate_config_file_rejects_missing_required_stratum_service() {
        let dir = unique_fixture_dir();
        fs::create_dir_all(&dir).expect("fixture dir should be created");
        let path = dir.join("belter.toml");
        fs::write(
            &path,
            r#"
[service.bitcoind]
manager = "launchd"
unit = "system/com.bitcoind.node"

[service.mempool]
manager = "podman_compose"
compose_file = "/tmp/mempool.yml"
"#,
        )
        .expect("config should be written");

        let env = FixedEnvResolver::new(HashMap::new());
        let err =
            validate_config_file(&env, &path, false).expect_err("config should fail validation");
        assert!(err.to_string().contains("missing required service `stratum`"));

        remove_fixture_dir(&dir);
    }

    #[test]
    fn validate_config_file_rejects_unresolved_placeholders() {
        let dir = unique_fixture_dir();
        fs::create_dir_all(&dir).expect("fixture dir should be created");
        let path = dir.join("belter.toml");
        fs::write(
            &path,
            r#"
[service.bitcoind]
manager = "launchd"
unit = "${BITCOIND_LAUNCHD_UNIT}"

[service.stratum]
manager = "launchd"
unit = "${STRATUM_LAUNCHD_UNIT}"

[service.mempool]
manager = "podman_compose"
compose_file = "/tmp/mempool.yml"
"#,
        )
        .expect("config should be written");

        let env = FixedEnvResolver::new(HashMap::new());
        let err =
            validate_config_file(&env, &path, false).expect_err("config should fail validation");
        let rendered = err.to_string();
        let mentions_bitcoind = rendered.contains("BITCOIND_LAUNCHD_UNIT");
        let mentions_stratum = rendered.contains("STRATUM_LAUNCHD_UNIT");
        assert!(mentions_bitcoind || mentions_stratum);
        assert!(rendered.contains("requires missing env var"));
        assert!(rendered.contains("example: `export"));

        remove_fixture_dir(&dir);
    }

    #[test]
    fn validate_config_file_write_missing_appends_required_services() {
        let dir = unique_fixture_dir();
        fs::create_dir_all(&dir).expect("fixture dir should be created");
        let path = dir.join("belter.toml");
        fs::write(
            &path,
            r#"
[service.bitcoind]
manager = "launchd"
unit = "${BITCOIND_LAUNCHD_UNIT}"
"#,
        )
        .expect("config should be written");

        let env = FixedEnvResolver::new(HashMap::from([
            (
                "BITCOIND_LAUNCHD_UNIT".to_string(),
                "system/com.bitcoind.node".to_string(),
            ),
            (
                "STRATUM_LAUNCHD_UNIT".to_string(),
                "system/io.btc.public-pool".to_string(),
            ),
            (
                "MEMPOOL_COMPOSE_FILE".to_string(),
                "/tmp/mempool.yml".to_string(),
            ),
            (
                "MEMPOOL_COMPOSE_OVERRIDE".to_string(),
                "/tmp/mempool.override.yml".to_string(),
            ),
            ("MEMPOOL_PROJECT".to_string(), "mempool".to_string()),
            ("MEMPOOL_ENV_FILE".to_string(), "/tmp/mempool.env".to_string()),
        ]));

        let message = validate_config_file(&env, &path, true).expect("config should validate");
        assert!(message.contains("auto-added missing"));

        let written = fs::read_to_string(&path).expect("config file should be readable");
        assert!(written.contains("[service.stratum]"));
        assert!(written.contains("[service.mempool]"));

        remove_fixture_dir(&dir);
    }

    fn unique_fixture_dir() -> PathBuf {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be monotonic")
            .as_nanos();
        std::env::temp_dir().join(format!("belter-config-test-{ts}"))
    }

    fn remove_fixture_dir(dir: &PathBuf) {
        match fs::remove_dir_all(dir) {
            Ok(()) => {}
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            Err(err) => panic!("fixture dir should be removed: {err}"),
        }
    }
}
