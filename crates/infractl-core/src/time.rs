use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

pub fn now_utc_rfc3339() -> String {
    let now = OffsetDateTime::now_utc();
    now.format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}
