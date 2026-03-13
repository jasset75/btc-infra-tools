use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SeverityLevel {
    Debug,
    Info,
    Warning,
    Error,
    Fatal,
}

#[derive(Debug, Serialize)]
pub struct OutputEnvelope {
    pub ts: String,
    pub command: String,
    pub status: String,
    pub message: String,
    pub dry_run: bool,
    pub data: Value,
    pub events: Vec<OutputEvent>,
}

#[derive(Debug, Serialize)]
pub struct OutputEvent {
    pub ts: String,
    pub level: SeverityLevel,
    pub code: String,
    pub message: String,
    pub details: Value,
}
