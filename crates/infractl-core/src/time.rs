use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

pub fn now_utc_rfc3339() -> String {
    let now = OffsetDateTime::now_utc();
    now.format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

/// Provides the current time to callers that should not depend on the global system clock.
pub trait Clock {
    fn now_utc_rfc3339(&self) -> String;
}

/// Production clock backed by the host system UTC time.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_utc_rfc3339(&self) -> String {
        now_utc_rfc3339()
    }
}

pub struct FixedClock {
    now: String,
}

impl FixedClock {
    /// Creates a clock that always returns the same instant.
    pub fn new(now: impl Into<String>) -> Self {
        Self { now: now.into() }
    }
}

impl Clock for FixedClock {
    fn now_utc_rfc3339(&self) -> String {
        self.now.clone()
    }
}
