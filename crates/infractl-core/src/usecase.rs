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

        if service.manager != "launchd" {
            bail!(
                "service `{}` uses unsupported manager `{}`",
                self.service_name,
                service.manager
            );
        }

        let operation = self.launchd_operation(service, resolver)?;
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
    fn request_rejects_unsupported_manager() {
        let mut services = HashMap::new();
        services.insert(
            "unknown".to_string(),
            ServiceConfig {
                manager: "systemd".to_string(),
                unit: Some("foo".to_string()),
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
