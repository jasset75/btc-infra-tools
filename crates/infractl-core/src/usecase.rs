use crate::config::BelterConfig;
use crate::env::{EnvResolver, expand_placeholders};
use crate::plan::{Operation, Plan};
use anyhow::{Result, bail};

pub struct RestartServiceRequest<'a> {
    pub config: &'a BelterConfig,
    pub service_name: &'a str,
}

impl<'a> RestartServiceRequest<'a> {
    pub fn plan(&self, resolver: &dyn EnvResolver) -> Result<Plan> {
        let services = self
            .config
            .service
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing [service] section"))?;

        let service = services.get(self.service_name).ok_or_else(|| {
            anyhow::anyhow!("service `{}` not found in config", self.service_name)
        })?;

        if service.manager.trim().is_empty() {
            bail!("service `{}` has an empty `manager`", self.service_name);
        }

        let unit = service
            .unit
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("service `{}` is missing `unit`", self.service_name))?;

        let resolved_unit = expand_placeholders(unit, resolver)?;

        Ok(Plan {
            operations: vec![Operation::RestartService {
                manager: service.manager.clone(),
                unit: resolved_unit,
            }],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ServiceConfig;
    use crate::env::FixedEnvResolver;
    use std::collections::HashMap;

    #[test]
    fn test_restart_service_request_plan() {
        let mut services = HashMap::new();
        services.insert(
            "bitcoind".to_string(),
            ServiceConfig {
                manager: "launchd".to_string(),
                unit: Some("system/com.bitcoind.node".to_string()),
            },
        );
        let config = BelterConfig {
            service: Some(services),
        };

        let req = RestartServiceRequest {
            config: &config,
            service_name: "bitcoind",
        };

        let resolver = FixedEnvResolver::new(HashMap::new());

        let plan = req.plan(&resolver).expect("Failed to create plan");
        assert_eq!(plan.operations.len(), 1);
        assert_eq!(
            plan.operations[0],
            Operation::RestartService {
                manager: "launchd".to_string(),
                unit: "system/com.bitcoind.node".to_string()
            }
        );
    }

    #[test]
    fn test_restart_service_request_rejects_empty_manager() {
        let mut services = HashMap::new();
        services.insert(
            "bitcoind".to_string(),
            ServiceConfig {
                manager: " ".to_string(),
                unit: Some("system/com.bitcoind.node".to_string()),
            },
        );
        let config = BelterConfig {
            service: Some(services),
        };

        let req = RestartServiceRequest {
            config: &config,
            service_name: "bitcoind",
        };

        let resolver = FixedEnvResolver::new(HashMap::new());
        let err = req.plan(&resolver).unwrap_err();
        assert!(err.to_string().contains("has an empty `manager`"));
    }

    #[test]
    fn test_restart_service_request_rejects_missing_service_section() {
        let config = BelterConfig { service: None };
        let req = RestartServiceRequest {
            config: &config,
            service_name: "bitcoind",
        };

        let resolver = FixedEnvResolver::new(HashMap::new());
        let err = req.plan(&resolver).unwrap_err();
        assert!(err.to_string().contains("missing [service] section"));
    }

    #[test]
    fn test_restart_service_request_rejects_unknown_service() {
        let config = BelterConfig {
            service: Some(HashMap::new()),
        };
        let req = RestartServiceRequest {
            config: &config,
            service_name: "bitcoind",
        };

        let resolver = FixedEnvResolver::new(HashMap::new());
        let err = req.plan(&resolver).unwrap_err();
        assert!(
            err.to_string()
                .contains("service `bitcoind` not found in config")
        );
    }

    #[test]
    fn test_restart_service_request_rejects_missing_unit() {
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

        let req = RestartServiceRequest {
            config: &config,
            service_name: "bitcoind",
        };

        let resolver = FixedEnvResolver::new(HashMap::new());
        let err = req.plan(&resolver).unwrap_err();
        assert!(
            err.to_string()
                .contains("service `bitcoind` is missing `unit`")
        );
    }
}
