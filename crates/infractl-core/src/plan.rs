use serde::Serialize;

/// A concrete action that can be executed by an infrastructure adapter.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Operation {
    StartLaunchdService {
        unit: String,
    },
    StopLaunchdService {
        unit: String,
    },
    RestartLaunchdService {
        unit: String,
    },
    StartPodmanComposeService {
        compose_file: String,
        compose_override: Option<String>,
        project: Option<String>,
    },
    StopPodmanComposeService {
        compose_file: String,
        compose_override: Option<String>,
        project: Option<String>,
    },
    RestartPodmanComposeService {
        compose_file: String,
        compose_override: Option<String>,
        project: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExecutionReport {
    pub operation_index: usize,
    pub details: ExecutionDetails,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExecutionDetails {
    LaunchdRestartPidChange {
        unit: String,
        pid_before: Option<i32>,
        pid_after: Option<i32>,
    },
}

/// An ordered set of operations produced by a use case.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Plan {
    pub operations: Vec<Operation>,
}

/// Executes a plan against a concrete runtime environment.
pub trait Executor {
    fn execute(&mut self, plan: &Plan) -> anyhow::Result<Vec<ExecutionReport>>;
}
