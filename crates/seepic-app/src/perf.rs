//! Optional timing traces, enabled with `IV_TRACE_PERF=1`.
//!
//! Page-flip cost is split across three very different places: the background
//! decode, the main-thread work that turns pixels into a drawable texture, and
//! whatever the render pass itself does. Guessing which one dominates is how
//! you end up optimising the wrong one, so each is timed separately and printed
//! on one line per event.

/// Whether timing traces are on.
pub fn enabled() -> bool {
    std::env::var_os("IV_TRACE_PERF").is_some()
}

/// Milliseconds elapsed since `start`.
pub fn millis_of(start: std::time::Instant) -> f32 {
    start.elapsed().as_secs_f32() * 1000.
}

/// When the first trace line was written.
///
/// Every line carries the time since then, because the interesting question is
/// almost always "how long after the key stopped did this happen", and the
/// answer is otherwise only available by guessing at wall-clock gaps.
pub fn origin() -> std::time::Instant {
    static ORIGIN: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    *ORIGIN.get_or_init(std::time::Instant::now)
}

/// Print one trace line, prefixed so the lines can be filtered out of stderr.
#[macro_export]
macro_rules! perf {
    ($($arg:tt)*) => {
        if $crate::perf::enabled() {
            eprintln!(
                "perf +{:.0}ms {}",
                $crate::perf::millis_of($crate::perf::origin()),
                format_args!($($arg)*)
            );
        }
    };
}
