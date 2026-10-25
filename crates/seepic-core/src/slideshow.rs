//! Slideshow timing and playback state.
//!
//! Whole-second presets (1–9) are offered for convenience, but the interval is
//! stored canonically in milliseconds, so any value down to 1 ms can be set.

use std::time::Duration;

/// The 1–9 second presets, bound to `Ctrl+1`..`Ctrl+9`.
pub const PRESET_SECONDS: [u64; 9] = [1, 2, 3, 4, 5, 6, 7, 8, 9];

/// Which unit the user is typing in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum IntervalUnit {
    #[default]
    Milliseconds,
    Seconds,
}

impl IntervalUnit {
    pub fn suffix(self) -> &'static str {
        match self {
            IntervalUnit::Milliseconds => "毫秒",
            IntervalUnit::Seconds => "秒",
        }
    }

    /// Short suffix for compact labels such as `250ms`.
    pub fn short_suffix(self) -> &'static str {
        match self {
            IntervalUnit::Milliseconds => "ms",
            IntervalUnit::Seconds => "s",
        }
    }
}

/// How long each slide stays on screen.
///
/// Always stored as milliseconds so that the value is exact and serialization
/// is lossless.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Interval {
    ms: u64,
}

impl Interval {
    /// Below this, decoding cannot keep up on any realistic machine.
    pub const MIN_MS: u64 = 1;
    /// 9999 seconds, which is well past anything usable.
    pub const MAX_MS: u64 = 9_999_000;

    /// Build from a millisecond count, clamped into the supported range.
    pub fn from_millis(ms: u64) -> Self {
        Self {
            ms: ms.clamp(Self::MIN_MS, Self::MAX_MS),
        }
    }

    /// Build from a value the user typed plus the unit they typed it in.
    pub fn from_parts(value: f64, unit: IntervalUnit) -> Self {
        let ms = match unit {
            IntervalUnit::Milliseconds => value,
            IntervalUnit::Seconds => value * 1000.,
        };
        // A NaN or negative entry falls back to the floor rather than panicking.
        if ms.is_finite() && ms > 0. {
            Self::from_millis(ms.round() as u64)
        } else {
            Self::from_millis(Self::MIN_MS)
        }
    }

    /// Build from one of [`PRESET_SECONDS`].
    pub fn from_preset(seconds: u64) -> Self {
        Self::from_millis(seconds.saturating_mul(1000))
    }

    pub fn as_millis(&self) -> u64 {
        self.ms
    }

    pub fn as_duration(&self) -> Duration {
        Duration::from_millis(self.ms)
    }

    /// Split back into a number and unit for display, preferring whole seconds
    /// when the value is an exact multiple of 1000 ms.
    pub fn suggest_parts(&self) -> (f64, IntervalUnit) {
        if self.ms % 1000 == 0 {
            ((self.ms / 1000) as f64, IntervalUnit::Seconds)
        } else {
            (self.ms as f64, IntervalUnit::Milliseconds)
        }
    }

    /// Which preset this equals, if any, as a 1-based second count.
    pub fn preset_seconds(&self) -> Option<u64> {
        if self.ms % 1000 == 0 {
            let secs = self.ms / 1000;
            PRESET_SECONDS.contains(&secs).then_some(secs)
        } else {
            None
        }
    }

    /// Label used in the slideshow popup, e.g. `1 秒(s)` or `自定义 250 毫秒`.
    pub fn label(&self) -> String {
        match self.preset_seconds() {
            Some(secs) => format!("{secs} 秒(s)"),
            None => format!("自定义 {}", self.compact()),
        }
    }

    /// Compact value with unit, e.g. `250ms`, `1.5s` or `2s`.
    ///
    /// Seconds are used once the value reaches a second, because `1500ms` is
    /// harder to read at a glance than `1.5s`.
    pub fn compact(&self) -> String {
        if self.ms < 1000 {
            return format!("{}ms", self.ms);
        }

        let secs = self.ms as f64 / 1000.;
        if secs.fract() == 0. {
            format!("{}s", secs as u64)
        } else {
            let text = format!("{secs:.3}");
            format!("{}s", text.trim_end_matches('0').trim_end_matches('.'))
        }
    }
}

impl Default for Interval {
    fn default() -> Self {
        Self::from_preset(1)
    }
}

/// Transition played when moving to the next slide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum PageTurnFx {
    #[default]
    None,
    Flip,
    Fade,
    Slide,
}

impl PageTurnFx {
    pub const ALL: [PageTurnFx; 4] = [
        PageTurnFx::None,
        PageTurnFx::Flip,
        PageTurnFx::Fade,
        PageTurnFx::Slide,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PageTurnFx::None => "无",
            PageTurnFx::Flip => "翻页",
            PageTurnFx::Fade => "淡入",
            PageTurnFx::Slide => "滑动",
        }
    }
}

/// User-facing slideshow settings, persisted verbatim.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SlideshowConfig {
    /// The gap after a slide has been shown, before the next one arrives.
    ///
    /// This is also how long the page-turn effect has to play.
    pub interval: Interval,
    /// How long each slide is displayed at rest.
    pub display: Interval,
    pub loop_playback: bool,
    pub random: bool,
    pub show_progress: bool,
    pub page_turn_fx: PageTurnFx,
}

impl Default for SlideshowConfig {
    fn default() -> Self {
        Self {
            interval: Interval::default(),
            display: Interval::default(),
            loop_playback: true,
            random: false,
            show_progress: false,
            page_turn_fx: PageTurnFx::None,
        }
    }
}

/// What the player should do after a slide's time is up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Advance {
    /// Show this index next.
    To(usize),
    /// Playback is finished; stop.
    Stop,
}

/// Playback state. Timing itself is driven by the UI layer, which owns the
/// timer; this type only decides *what* happens next and *how long* to wait.
#[derive(Debug, Clone)]
pub struct Slideshow {
    config: SlideshowConfig,
    running: bool,
    /// How many slides we have advanced through since starting, used to stop a
    /// random slideshow after one full pass when looping is off.
    advanced: usize,
    rng: Rng,
}

impl Slideshow {
    pub fn new(config: SlideshowConfig) -> Self {
        Self {
            config,
            running: false,
            advanced: 0,
            rng: Rng::seeded(),
        }
    }

    pub fn config(&self) -> &SlideshowConfig {
        &self.config
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    /// How long to show each slide.
    pub fn interval(&self) -> Duration {
        self.config.interval.as_duration()
    }

    pub fn start(&mut self) {
        self.running = true;
        self.advanced = 0;
    }

    pub fn stop(&mut self) {
        self.running = false;
        self.advanced = 0;
    }

    /// Start if stopped, stop if running.
    pub fn toggle(&mut self) {
        if self.running {
            self.stop();
        } else {
            self.start();
        }
    }

    /// Change the interval while playing; the caller restarts its timer.
    pub fn set_interval(&mut self, interval: Interval) {
        self.config.interval = interval;
    }

    /// How long each slide is shown before the gap starts.
    pub fn display(&self) -> Duration {
        self.config.display.as_duration()
    }

    pub fn set_display(&mut self, interval: Interval) {
        self.config.display = interval;
    }

    /// One full slide: the display time plus the gap before the next.
    pub fn cycle(&self) -> Duration {
        self.display() + self.interval()
    }

    pub fn set_loop_playback(&mut self, value: bool) {
        self.config.loop_playback = value;
    }

    pub fn set_random(&mut self, value: bool) {
        self.config.random = value;
    }

    pub fn set_show_progress(&mut self, value: bool) {
        self.config.show_progress = value;
    }

    pub fn set_page_turn_fx(&mut self, fx: PageTurnFx) {
        self.config.page_turn_fx = fx;
    }

    /// Pick the slide to show after `current`, or stop.
    ///
    /// A random slideshow walks a full pass before stopping when looping is
    /// off, so that it cannot spin forever on a two-image folder.
    pub fn advance(&mut self, current: usize, total: usize) -> Advance {
        if total == 0 {
            return Advance::Stop;
        }

        self.advanced += 1;

        if self.config.random {
            if !self.config.loop_playback && self.advanced >= total {
                return Advance::Stop;
            }
            return Advance::To(self.random_other_than(current, total));
        }

        let next = current + 1;
        if next < total {
            return Advance::To(next);
        }
        if self.config.loop_playback {
            Advance::To(0)
        } else {
            Advance::Stop
        }
    }

    /// Fraction of the current slide's time that has elapsed, clamped to 0..=1.
    /// Drives the progress bar.
    pub fn progress(&self, elapsed: Duration) -> f32 {
        let total = self.cycle().as_millis().max(1) as f64;
        let done = elapsed.as_millis() as f64;
        (done / total).clamp(0., 1.) as f32
    }

    fn random_other_than(&mut self, current: usize, total: usize) -> usize {
        if total <= 1 {
            return 0;
        }
        // Draw from the other `total - 1` slides so we never repeat the current
        // one, which would look like a stall.
        let pick = self.rng.below(total as u64 - 1) as usize;
        if pick >= current { pick + 1 } else { pick }
    }
}

/// A small xorshift generator, so the core crate needs no RNG dependency.
#[derive(Debug, Clone)]
struct Rng(u64);

impl Rng {
    fn seeded() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x2545_F491_4F6C_DD1D);
        Self(nanos | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Uniform-ish value in `0..bound`. `bound` must be non-zero.
    fn below(&mut self, bound: u64) -> u64 {
        self.next_u64() % bound
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_round_trips_through_units() {
        assert_eq!(Interval::from_parts(250., IntervalUnit::Milliseconds).as_millis(), 250);
        assert_eq!(Interval::from_parts(1.5, IntervalUnit::Seconds).as_millis(), 1500);
        assert_eq!(Interval::from_parts(2., IntervalUnit::Seconds).as_millis(), 2000);
    }

    #[test]
    fn interval_clamps_and_rejects_nonsense() {
        assert_eq!(Interval::from_millis(0).as_millis(), Interval::MIN_MS);
        assert_eq!(Interval::from_millis(u64::MAX).as_millis(), Interval::MAX_MS);
        assert_eq!(Interval::from_parts(-5., IntervalUnit::Seconds).as_millis(), Interval::MIN_MS);
        assert_eq!(Interval::from_parts(f64::NAN, IntervalUnit::Seconds).as_millis(), Interval::MIN_MS);
    }

    #[test]
    fn presets_are_recognised() {
        assert_eq!(Interval::from_preset(3).preset_seconds(), Some(3));
        assert_eq!(Interval::from_millis(3000).preset_seconds(), Some(3));
        assert_eq!(Interval::from_millis(1500).preset_seconds(), None);
        assert_eq!(Interval::from_millis(10_000).preset_seconds(), None);
    }

    #[test]
    fn labels_match_the_popup() {
        assert_eq!(Interval::from_preset(1).label(), "1 秒(s)");
        assert_eq!(Interval::from_millis(250).label(), "自定义 250ms");
        assert_eq!(Interval::from_millis(1500).label(), "自定义 1.5s");
    }

    #[test]
    fn compact_switches_to_seconds_from_a_second_up() {
        assert_eq!(Interval::from_millis(1).compact(), "1ms");
        assert_eq!(Interval::from_millis(999).compact(), "999ms");
        assert_eq!(Interval::from_millis(1000).compact(), "1s");
        assert_eq!(Interval::from_millis(1250).compact(), "1.25s");
        assert_eq!(Interval::from_millis(2000).compact(), "2s");
        assert_eq!(Interval::from_millis(10_000).compact(), "10s");
    }

    #[test]
    fn suggested_parts_prefer_whole_seconds() {
        assert_eq!(Interval::from_millis(3000).suggest_parts(), (3.0, IntervalUnit::Seconds));
        assert_eq!(
            Interval::from_millis(250).suggest_parts(),
            (250.0, IntervalUnit::Milliseconds)
        );
    }

    #[test]
    fn sequential_advance_wraps_only_when_looping() {
        let mut show = Slideshow::new(SlideshowConfig::default());
        show.start();
        assert_eq!(show.advance(0, 3), Advance::To(1));
        assert_eq!(show.advance(2, 3), Advance::To(0));

        show.set_loop_playback(false);
        assert_eq!(show.advance(1, 3), Advance::To(2));
        assert_eq!(show.advance(2, 3), Advance::Stop);
    }

    #[test]
    fn empty_container_stops() {
        let mut show = Slideshow::new(SlideshowConfig::default());
        show.start();
        assert_eq!(show.advance(0, 0), Advance::Stop);
    }

    #[test]
    fn random_never_repeats_current() {
        let mut config = SlideshowConfig::default();
        config.random = true;
        let mut show = Slideshow::new(config);
        show.start();

        for _ in 0..500 {
            for current in 0..5 {
                let Advance::To(next) = show.advance(current, 5) else {
                    panic!("random playback should not stop while looping");
                };
                assert_ne!(next, current, "random advance repeated the current slide");
                assert!(next < 5);
            }
        }
    }

    #[test]
    fn random_without_loop_stops_after_one_pass() {
        let mut config = SlideshowConfig::default();
        config.random = true;
        config.loop_playback = false;
        let mut show = Slideshow::new(config);
        show.start();

        let mut seen = 0;
        loop {
            match show.advance(seen % 4, 4) {
                Advance::To(_) => seen += 1,
                Advance::Stop => break,
            }
        }
        assert_eq!(seen, 3, "a 4-image random pass should advance 3 times then stop");
    }

    #[test]
    fn single_image_random_does_not_panic() {
        let mut config = SlideshowConfig::default();
        config.random = true;
        let mut show = Slideshow::new(config);
        show.start();
        assert_eq!(show.advance(0, 1), Advance::To(0));
    }

    #[test]
    fn progress_covers_the_whole_cycle() {
        // The cycle is the display time plus the gap before the next slide.
        let show = Slideshow::new(SlideshowConfig {
            interval: Interval::from_millis(100),
            display: Interval::from_millis(100),
            ..Default::default()
        });
        assert_eq!(show.cycle(), Duration::from_millis(200));
        assert_eq!(show.progress(Duration::from_millis(0)), 0.0);
        assert_eq!(show.progress(Duration::from_millis(100)), 0.5);
        assert_eq!(show.progress(Duration::from_millis(200)), 1.0);
        assert_eq!(show.progress(Duration::from_millis(900)), 1.0);
    }

    #[test]
    fn both_durations_default_to_one_second() {
        let config = SlideshowConfig::default();
        assert_eq!(config.interval.as_millis(), 1000);
        assert_eq!(config.display.as_millis(), 1000);
    }

    #[test]
    fn config_round_trips_through_json() {
        let config = SlideshowConfig {
            interval: Interval::from_millis(250),
            random: true,
            page_turn_fx: PageTurnFx::Fade,
            ..Default::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("250"), "interval should serialize as plain ms: {json}");
        assert_eq!(serde_json::from_str::<SlideshowConfig>(&json).unwrap(), config);
    }
}
