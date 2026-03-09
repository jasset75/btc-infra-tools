use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct OutputEnvelope {
    pub ts: String,
    pub command: String,
    pub status: String,
    pub message: String,
}
