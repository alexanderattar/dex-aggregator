pub mod event_processor;
mod matching;
pub mod request_processor;
mod routing;
pub mod state;
#[cfg(test)]
mod tests;

use std::time::Duration;

/// Format a duration with appropriate units for readability.
/// Automatically scales to ns, μs, ms, or s based on magnitude.
pub(crate) fn format_duration(d: Duration) -> String {
    let nanos = d.as_nanos();
    if nanos < 1_000 {
        format!("{}ns", nanos)
    } else if nanos < 1_000_000 {
        format!("{:.1}μs", nanos as f64 / 1_000.0)
    } else if nanos < 1_000_000_000 {
        format!("{:.2}ms", nanos as f64 / 1_000_000.0)
    } else {
        format!("{:.2}s", d.as_secs_f64())
    }
}
