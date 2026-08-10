//! Timestamp helpers (RFC 3339 / ISO 8601 UTC strings).

use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

pub fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

pub fn system_time_rfc3339(value: std::time::SystemTime) -> Option<String> {
    let duration = value.duration_since(std::time::UNIX_EPOCH).ok()?;
    let datetime = OffsetDateTime::from_unix_timestamp(duration.as_secs() as i64).ok()?;
    datetime.format(&Rfc3339).ok()
}
