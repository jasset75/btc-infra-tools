use crate::config::{BelterConfig, ServiceConfig};
use crate::env::{EnvResolver, expand_placeholders};
use crate::plan::{Operation, Plan};
use anyhow::{Result, anyhow, bail};

#[derive(Debug, Clone, Copy)]
pub enum ServiceAction {
    Start,
    Stop,
    Restart,
}

pub struct ServiceCommandRequest<'a> {
    pub config: &'a BelterConfig,
    pub service_name: &'a str,
    pub action: ServiceAction,
}

impl<'a> ServiceCommandRequest<'a> {
    pub fn plan(&self, resolver: &dyn EnvResolver) -> Result<Plan> {
        let services = self
            .config
            .service
            .as_ref()
            .ok_or_else(|| anyhow!("missing [service] section"))?;

        let service = services.get(self.service_name).ok_or_else(|| {
            anyhow!("service `{}` not found in config", self.service_name)
        })?;

        if service.manager.trim().is_empty() {
            bail!("service `{}` has an empty `manager`", self.service_name);
        }

        let operation = match service.manager.as_str() {
            "launchd" => self.launchd_operation(service, resolver)?,
            "podman_compose" => self.podman_compose_operation(service, resolver)?,
            other => bail!(
                "service `{}` uses unsupported manager `{other}`",
                self.service_name
            ),
        };

        Ok(Plan {
            operations: vec![operation],
        })
    }

    fn launchd_operation(
        &self,
        service: &ServiceConfig,
        resolver: &dyn EnvResolver,
    ) -> Result<Operation> {
        let unit = service
            .unit
            .as_deref()
            .ok_or_else(|| anyhow!("service `{}` is missing `unit`", self.service_name))?;
        let unit = expand_placeholders(unit, resolver)?;

        Ok(match self.action {
            ServiceAction::Start => Operation::StartLaunchdService { unit },
            ServiceAction::Stop => Operation::StopLaunchdService { unit },
            ServiceAction::Restart => Operation::RestartLaunchdService { unit },
        })
    }

    fn podman_compose_operation(
        &self,
        service: &ServiceConfig,
        resolver: &dyn EnvResolver,
    ) -> Result<Operation> {
        let compose_file = service.compose_file.as_deref().ok_or_else(|| {
            anyhow!(
                "service `{}` is missing `compose_file`",
                self.service_name
            )
        })?;
        let compose_file = expand_placeholders(compose_file, resolver)?;

        let compose_override = service
            .compose_override
            .as_deref()
            .map(|value| expand_placeholders(value, resolver))
            .transpose()?;
        let project = service
            .project
            .as_deref()
            .map(|value| expand_placeholders(value, resolver))
            .transpose()?;

        Ok(match self.action {
            ServiceAction::Start => Operation::StartPodmanComposeService {
                compose_file,
                compose_override,
                project,
            },
            ServiceAction::Stop => Operation::StopPodmanComposeService {
                compose_file,
                compose_override,
                project,
            },
            ServiceAction::Restart => Operation::RestartPodmanComposeService {
                compose_file,
                compose_override,
                project,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::FixedEnvResolver;
    use std::collections::HashMap;

    fn launchd_config() -> BelterConfig {
        let mut services = HashMap::new();
        services.insert(
            "bitcoind".to_string(),
            ServiceConfig {
                manager: "launchd".to_string(),
                unit: Some("system/com.bitcoind.node".to_string()),
                compose_file: None,
                compose_override: None,
                project: None,
            },
        );
        BelterConfig {
            service: Some(services),
        }
    }

    #[test]
    fn restart_launchd_service_request_plan() {
        let config = launchd_config();
        let req = ServiceCommandRequest {
            config: &config,
            service_name: "bitcoind",
            action: ServiceAction::Restart,
        };

        let resolver = FixedEnvResolver::new(HashMap::new());
        let plan = req.plan(&resolver).expect("failed to create plan");
        assert_eq!(
            plan.operations,
            vec![Operation::RestartLaunchdService {
                unit: "system/com.bitcoind.node".to_string()
            }]
        );
    }

    #[test]
    fn start_podman_compose_service_request_plan() {
        let mut services = HashMap::new();
        services.insert(
            "mempool".to_string(),
            ServiceConfig {
                manager: "podman_compose".to_string(),
                unit: None,
                compose_file: Some("${MEMPOOL_COMPOSE_FILE}".to_string()),
                compose_override: Some("${MEMPOOL_COMPOSE_OVERRIDE}".to_string()),
                project: Some("${MEMPOOL_PROJECT}".to_string()),
            },
        );
        let config = BelterConfig {
            service: Some(services),
        };

        let resolver = FixedEnvResolver::new(HashMap::from([
            (
                "MEMPOOL_COMPOSE_FILE".to_string(),
                "/tmp/base.yml".to_string(),
            ),
            (
                "MEMPOOL_COMPOSE_OVERRIDE".to_string(),
                "/tmp/override.yml".to_string(),
            ),
            ("MEMPOOL_PROJECT".to_string(), "docker".to_string()),
        ]));

        let req = ServiceCommandRequest {
            config: &config,
            service_name: "mempool",
            action: ServiceAction::Start,
        };

        let plan = req.plan(&resolver).expect("failed to create plan");
        assert_eq!(
            plan.operations,
            vec![Operation::StartPodmanComposeService {
                compose_file: "/tmp/base.yml".to_string(),
                compose_override: Some("/tmp/override.yml".to_string()),
                project: Some("docker".to_string()),
            }]
        );
    }

    #[test]
    fn request_rejects_missing_service_section() {
        let config = BelterConfig { service: None };
        let req = ServiceCommandRequest {
            config: &config,
            service_name: "bitcoind",
            action: ServiceAction::Restart,
        };

        let resolver = FixedEnvResolver::new(HashMap::new());
        let err = req.plan(&resolver).unwrap_err();
        assert!(err.to_string().contains("missing [service] section"));
    }

    #[test]
    fn request_rejects_unknown_service() {
        let config = BelterConfig {
            service: Some(HashMap::new()),
        };
        let req = ServiceCommandRequest {
            config: &config,
            service_name: "bitcoind",
            action: ServiceAction::Restart,
        };

        let resolver = FixedEnvResolver::new(HashMap::new());
        let err = req.plan(&resolver).unwrap_err();
        assert!(
            err.to_string()
                .contains("service `bitcoind` not found in config")
        );
    }

    #[test]
    fn request_rejects_missing_unit_for_launchd() {
        let mut services = HashMap::new();
        services.insert(
            "bitcoind".to_string(),
            ServiceConfig {
                manager: "launchd".to_string(),
                unit: None,
                compose_file: None,
                compose_override: None,
                project: None,
            },
        );
        let config = BelterConfig {
            service: Some(services),
        };
        let req = ServiceCommandRequest {
            config: &config,
            service_name: "bitcoind",
            action: ServiceAction::Restart,
        };

        let resolver = FixedEnvResolver::new(HashMap::new());
        let err = req.plan(&resolver).unwrap_err();
        assert!(
            err.to_string()
                .contains("service `bitcoind` is missing `unit`")
        );
    }

    #[test]
    fn request_rejects_missing_compose_file_for_podman_compose() {
        let mut services = HashMap::new();
        services.insert(
            "mempool".to_string(),
            ServiceConfig {
                manager: "podman_compose".to_string(),
                unit: None,
                compose_file: None,
                compose_override: None,
                project: None,
            },
        );
        let config = BelterConfig {
            service: Some(services),
        };
        let req = ServiceCommandRequest {
            config: &config,
            service_name: "mempool",
            action: ServiceAction::Restart,
        };

        let resolver = FixedEnvResolver::new(HashMap::new());
        let err = req.plan(&resolver).unwrap_err();
        assert!(
            err.to_string()
                .contains("service `mempool` is missing `compose_file`")
        );
    }

    #[test]
    fn request_rejects_unsupported_manager() {
        let mut services = HashMap::new();
        services.insert(
            "unknown".to_string(),
            ServiceConfig {
                manager: "systemd".to_string(),
                unit: Some("foo".to_string()),
                compose_file: None,
                compose_override: None,
                project: None,
            },
        );
        let config = BelterConfig {
            service: Some(services),
        };
        let req = ServiceCommandRequest {
            config: &config,
            service_name: "unknown",
            action: ServiceAction::Restart,
        };

        let resolver = FixedEnvResolver::new(HashMap::new());
        let err = req.plan(&resolver).unwrap_err();
        assert!(
            err.to_string()
                .contains("service `unknown` uses unsupported manager `systemd`")
        );
    }
}
