//! Two finger swipe between notes: the offset that follows the fingers,
//! rubber banding at the ends, the commit decision and the settle animation.
//!
//! Offsets are in pixels. A positive offset moves the current note left and
//! brings the newer note in from the right. Times are milliseconds from any
//! fixed start.

/// Horizontal wheel delta for one full note width. A tilt wheel notch (120)
/// is then a quarter of the width.
pub const DELTA_PER_WIDTH: f64 = 480.0;
/// A gesture ends after this long without a horizontal wheel event.
pub const END_MS: u64 = 150;
/// Commit to the neighbour when it is at least this much of the width in.
pub const COMMIT_FRACTION: f64 = 0.25;
/// Or when the recent speed toward it is at least this many widths per second.
pub const COMMIT_SPEED: f64 = 1.5;
/// Speed is measured over the events of this last stretch of time.
pub const SPEED_WINDOW_MS: u64 = 100;
/// Past the oldest note or the empty note, the offset moves this many times
/// slower than the fingers...
pub const RUBBER_BAND_DIVISOR: f64 = 3.0;
/// ...and stops at this fraction of the width.
pub const RUBBER_BAND_MAX: f64 = 0.15;
/// Length of the animation to the neighbour or back.
pub const SETTLE_MS: u64 = 200;
/// Animation frame interval, near 60 Hz.
pub const FRAME_MS: u32 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Older,
    Newer,
}

impl Side {
    /// The sign of offsets toward this side.
    pub fn sign(self) -> f64 {
        match self {
            Side::Older => -1.0,
            Side::Newer => 1.0,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Side::Older => "older",
            Side::Newer => "newer",
        }
    }
}

/// A gesture while the fingers (or touchpad inertia) are moving.
pub struct Swipe {
    width: f64,
    can_older: bool,
    can_newer: bool,
    /// Wheel units so far, so that offsets add up exactly.
    units: i64,
    /// Offset following the fingers, before rubber banding.
    raw: f64,
    last_ms: u64,
    /// Recent (time, raw offset) samples, oldest first, for the speed.
    samples: Vec<(u64, f64)>,
}

impl Swipe {
    pub fn new(width: f64, can_older: bool, can_newer: bool, now: u64) -> Self {
        Swipe {
            width: width.max(1.0),
            can_older,
            can_newer,
            units: 0,
            raw: 0.0,
            last_ms: now,
            samples: vec![(now, 0.0)],
        }
    }

    /// Continues from an offset already on screen, as when an animation is
    /// interrupted by more wheel events.
    pub fn resume(width: f64, can_older: bool, can_newer: bool, offset: f64, now: u64) -> Self {
        let mut swipe = Swipe::new(width, can_older, can_newer, now);
        let free = offset == 0.0
            || swipe.can(if offset > 0.0 {
                Side::Newer
            } else {
                Side::Older
            });
        let raw = if free {
            offset
        } else {
            offset * RUBBER_BAND_DIVISOR
        };
        swipe.units = (raw * DELTA_PER_WIDTH / swipe.width).round() as i64;
        swipe.raw = swipe.raw_of_units();
        swipe.samples = vec![(now, swipe.raw)];
        swipe
    }

    pub fn can(&self, side: Side) -> bool {
        match side {
            Side::Older => self.can_older,
            Side::Newer => self.can_newer,
        }
    }

    /// Treats `side` as an end, for example when its note cannot be read.
    pub fn block(&mut self, side: Side) {
        match side {
            Side::Older => self.can_older = false,
            Side::Newer => self.can_newer = false,
        }
    }

    /// Adds a horizontal wheel delta; positive moves toward the newer note.
    pub fn scroll(&mut self, delta: i32, now: u64) {
        // Past a full width there is nothing more to reveal.
        let max = DELTA_PER_WIDTH as i64;
        self.units = (self.units + i64::from(delta)).clamp(-max, max);
        self.raw = self.raw_of_units();
        self.last_ms = now;
        self.samples.push((now, self.raw));
        // Keep the samples of the speed window, and at least two.
        let start = now.saturating_sub(SPEED_WINDOW_MS);
        while self.samples.len() > 2 && self.samples[0].0 < start {
            self.samples.remove(0);
        }
    }

    fn raw_of_units(&self) -> f64 {
        self.units as f64 * self.width / DELTA_PER_WIDTH
    }

    /// The side being revealed, if any.
    pub fn side(&self) -> Option<Side> {
        if self.raw > 0.0 {
            Some(Side::Newer)
        } else if self.raw < 0.0 {
            Some(Side::Older)
        } else {
            None
        }
    }

    /// The offset to show, with rubber banding toward a missing neighbour.
    pub fn offset(&self) -> f64 {
        match self.side() {
            Some(side) if !self.can(side) => {
                let banded = self.raw.abs() / RUBBER_BAND_DIVISOR;
                side.sign() * banded.min(RUBBER_BAND_MAX * self.width)
            }
            _ => self.raw,
        }
    }

    /// Whether the gesture has ended: no event for `END_MS`.
    pub fn ended(&self, now: u64) -> bool {
        now.saturating_sub(self.last_ms) >= END_MS
    }

    /// Recent speed in pixels per second, positive toward the newer note.
    pub fn speed(&self) -> f64 {
        let (t0, x0) = self.samples[0];
        let (t1, x1) = self.samples[self.samples.len() - 1];
        if t1 > t0 {
            (x1 - x0) * 1000.0 / (t1 - t0) as f64
        } else {
            0.0
        }
    }

    /// Where the gesture should settle: the neighbour on `Some(side)`, or
    /// back to the current note on `None`.
    pub fn decide(&self) -> Option<Side> {
        let side = self.side().filter(|&s| self.can(s))?;
        let toward = self.speed() * side.sign();
        let threshold = COMMIT_SPEED * self.width;
        if toward <= -threshold {
            // Flicked back.
            return None;
        }
        let distance = self.offset().abs() / self.width;
        (toward >= threshold || distance >= COMMIT_FRACTION).then_some(side)
    }

    /// Like `decide`, by distance alone: for a gesture cut short by a key or
    /// click, where the speed at that moment says nothing about the target.
    pub fn decide_by_distance(&self) -> Option<Side> {
        let side = self.side().filter(|&s| self.can(s))?;
        (self.offset().abs() / self.width >= COMMIT_FRACTION).then_some(side)
    }
}

/// The animation from the offset at the end of a gesture to its target.
pub struct Settle {
    pub from: f64,
    pub to: f64,
    pub start_ms: u64,
}

impl Settle {
    /// The offset at `now`, and whether the animation is over.
    pub fn at(&self, now: u64) -> (f64, bool) {
        let t = now.saturating_sub(self.start_ms) as f64 / SETTLE_MS as f64;
        if t >= 1.0 {
            return (self.to, true);
        }
        (self.from + (self.to - self.from) * ease_out(t), false)
    }
}

/// Cubic ease out: fast at first, slowing to a stop.
pub fn ease_out(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: f64 = 400.0;

    #[test]
    fn offset_follows_deltas() {
        let mut s = Swipe::new(W, true, true, 0);
        assert_eq!(s.side(), None);
        for i in 1..=12 {
            s.scroll(10, i * 8);
        }
        // 120 units of 480 per width.
        assert!((s.offset() - 100.0).abs() < 1e-9);
        assert_eq!(s.side(), Some(Side::Newer));
        s.scroll(-1000, 200);
        assert_eq!(s.offset(), -W, "clamped to a full width");
        assert_eq!(s.side(), Some(Side::Older));
    }

    #[test]
    fn direction_changes_switch_side() {
        let mut s = Swipe::new(W, true, true, 0);
        s.scroll(30, 8);
        assert_eq!(s.side(), Some(Side::Newer));
        s.scroll(-60, 16);
        assert_eq!(s.side(), Some(Side::Older));
        assert!((s.offset() + 25.0).abs() < 1e-9);
        s.scroll(30, 24);
        assert_eq!(s.side(), None);
    }

    #[test]
    fn rubber_band_at_the_ends() {
        let mut s = Swipe::new(W, true, false, 0);
        s.scroll(48, 8); // 40 px of finger travel
        assert!((s.offset() - 40.0 / RUBBER_BAND_DIVISOR).abs() < 1e-9);
        s.scroll(4800, 16);
        assert_eq!(s.offset(), RUBBER_BAND_MAX * W);
        assert_eq!(s.decide(), None, "never commits past the end");
        // The other side is free.
        let mut s = Swipe::new(W, true, false, 0);
        s.scroll(-48, 8);
        assert!((s.offset() + 40.0).abs() < 1e-9);
        // Blocking a side turns it into an end.
        s.block(Side::Older);
        assert!((s.offset() + 40.0 / RUBBER_BAND_DIVISOR).abs() < 1e-9);
    }

    #[test]
    fn commits_by_distance() {
        // Slow drags: 10 units every 100 ms, so speed is far below the threshold.
        let drag = |units: i32| {
            let mut s = Swipe::new(W, true, true, 0);
            for i in 1..=units / 10 {
                s.scroll(10, i as u64 * 100);
            }
            s
        };
        assert_eq!(drag(110).decide(), None);
        assert_eq!(drag(120).decide(), Some(Side::Newer));
        // One tilt wheel notch is exactly a quarter width.
        let mut s = Swipe::new(W, true, true, 0);
        s.scroll(-120, 0);
        assert_eq!(s.decide(), Some(Side::Older));
    }

    #[test]
    fn commits_by_speed() {
        // 40 units in 40 ms: 33 px in 40 ms is 833 px/s, over 1.5 * 400.
        let mut s = Swipe::new(W, true, true, 0);
        for i in 1..=4 {
            s.scroll(10, 1000 + i * 10);
        }
        assert!(s.offset() < COMMIT_FRACTION * W);
        assert!(s.speed() > COMMIT_SPEED * W);
        assert_eq!(s.decide(), Some(Side::Newer));
        // A quick flick back cancels even past the distance threshold.
        let mut s = Swipe::new(W, true, true, 0);
        s.scroll(200, 0);
        for i in 1..=4 {
            s.scroll(-10, 1000 + i * 10);
        }
        assert!(s.offset() > COMMIT_FRACTION * W);
        assert_eq!(s.decide(), None);
    }

    #[test]
    fn decide_by_distance_ignores_speed() {
        // A fast start that is still short: speed alone would commit.
        let mut s = Swipe::new(W, true, true, 0);
        for i in 1..=4 {
            s.scroll(10, 1000 + i * 10);
        }
        assert_eq!(s.decide(), Some(Side::Newer));
        assert_eq!(s.decide_by_distance(), None);
        // Past a quarter width it still commits.
        s.scroll(100, 1100);
        assert_eq!(s.decide_by_distance(), Some(Side::Newer));
        // Nothing to commit to at the end of the list.
        let mut s = Swipe::new(W, true, false, 0);
        s.scroll(200, 0);
        assert_eq!(s.decide_by_distance(), None);
    }

    #[test]
    fn speed_uses_recent_events_only() {
        let mut s = Swipe::new(W, true, true, 0);
        s.scroll(240, 10); // fast at first
        for i in 0..20 {
            s.scroll(1, 500 + i * 10);
        }
        // Over the last 100 ms: about 10 units, 8 px in 100 ms.
        assert!(s.speed() < 100.0, "{}", s.speed());
    }

    #[test]
    fn ends_after_a_quiet_stretch() {
        let mut s = Swipe::new(W, true, true, 1000);
        s.scroll(10, 1008);
        assert!(!s.ended(1008 + END_MS - 1));
        assert!(s.ended(1008 + END_MS));
        s.scroll(10, 1100);
        assert!(!s.ended(1008 + END_MS));
    }

    #[test]
    fn resume_keeps_the_offset_on_screen() {
        let s = Swipe::resume(W, true, true, 150.0, 0);
        assert_eq!(s.offset(), 150.0);
        let s = Swipe::resume(W, false, true, -20.0, 0);
        assert!((s.offset() + 20.0).abs() < 1e-9);
    }

    #[test]
    fn easing_and_settle() {
        assert_eq!(ease_out(0.0), 0.0);
        assert_eq!(ease_out(1.0), 1.0);
        assert!(ease_out(0.5) > 0.5, "eases out");
        let mut last = 0.0;
        for i in 1..=10 {
            let v = ease_out(f64::from(i) / 10.0);
            assert!(v > last);
            last = v;
        }
        let settle = Settle {
            from: 100.0,
            to: W,
            start_ms: 50,
        };
        assert_eq!(settle.at(50), (100.0, false));
        let (mid, done) = settle.at(50 + SETTLE_MS / 2);
        assert!(!done && mid > 250.0 && mid < W);
        assert_eq!(settle.at(50 + SETTLE_MS), (W, true));
        assert_eq!(settle.at(10_000), (W, true));
    }
}
