use serde::Serialize;

/// A concrete action that can be executed by an infrastructure adapter.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Operation {
    RestartService { manager: String, unit: String },
}

/// An ordered set of operations produced by a use case.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Plan {
    pub operations: Vec<Operation>,
}

/// Executes a plan against a concrete runtime environment.
pub trait Executor {
    fn execute(&mut self, plan: &Plan) -> anyhow::Result<()>;
}
