use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const MAX_RECOVERY_ATTEMPTS: u32 = 3;
const BACKOFF_BASE_DELAY_SECS: f64 = 1.0;
const BACKOFF_MAX_DELAY_SECS: f64 = 30.0;

pub const CONTINUATION_MESSAGE: &str = "Output limit hit. Continue directly from where you stopped. \
No recap, no repetition. Pick up mid-sentence if needed.";

#[derive(Debug, Default)]
pub struct RecoveryState {
    pub continuation_attempts: u32,
    pub compact_attempts: u32,
    pub transport_attempts: u32,
}

pub fn is_prompt_too_long_error(error_text: &str) -> bool {
    (error_text.contains("prompt") && error_text.contains("long"))
        || error_text.contains("overlong_prompt")
        || error_text.contains("too many tokens")
        || error_text.contains("context length")
}

pub fn is_transient_transport_error(error_text: &str) -> bool {
    [
        "timeout",
        "timed out",
        "rate limit",
        "too many requests",
        "unavailable",
        "connection",
        "overloaded",
        "temporarily",
        "econnreset",
        "broken pipe",
    ]
    .iter()
    .any(|needle| error_text.contains(needle))
}

pub fn backoff_delay(attempt: u32) -> Duration {
    let base = (BACKOFF_BASE_DELAY_SECS * 2f64.powi(attempt as i32)).min(BACKOFF_MAX_DELAY_SECS);
    let jitter = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| (duration.subsec_millis() % 1000) as f64 / 1000.0)
        .unwrap_or(0.0);
    Duration::from_secs_f64(base + jitter)
}
