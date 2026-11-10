//! Time sources. `Clock` is injectable for tests; `Hlc` orders events across
//! devices whose wall clocks disagree (hybrid logical clock).

use std::cmp::Ordering;
use std::sync::atomic::{AtomicI64, Ordering as AtomicOrdering};
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
use std::time::{SystemTime, UNIX_EPOCH};

/// Milliseconds since the Unix epoch.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}

/// The system wall clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    fn now_ms(&self) -> i64 {
        // The browser has no `SystemTime`; its clock is `Date.now()`.
        let ms = js_sys::Date::now();
        if ms.is_finite() && ms >= 0.0 {
            // Whole milliseconds since the epoch fit an i64 exactly.
            #[allow(clippy::cast_possible_truncation)]
            {
                ms as i64
            }
        } else {
            0
        }
    }

    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    fn now_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0)
    }
}

/// A clock set by tests.
#[derive(Debug, Default)]
pub struct FixedClock(AtomicI64);

impl FixedClock {
    pub fn new(ms: i64) -> Self {
        Self(AtomicI64::new(ms))
    }
    pub fn set(&self, ms: i64) {
        self.0.store(ms, AtomicOrdering::SeqCst);
    }
    pub fn advance(&self, ms: i64) {
        self.0.fetch_add(ms, AtomicOrdering::SeqCst);
    }
}

impl Clock for FixedClock {
    fn now_ms(&self) -> i64 {
        self.0.load(AtomicOrdering::SeqCst)
    }
}

/// A hybrid logical clock timestamp: wall time, a counter, and the device
/// as tie-breaker. Total order, consistent with causality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HlcTimestamp {
    pub wall_ms: i64,
    pub counter: u32,
    pub node: u64,
}

impl Ord for HlcTimestamp {
    fn cmp(&self, o: &Self) -> Ordering {
        (self.wall_ms, self.counter, self.node).cmp(&(o.wall_ms, o.counter, o.node))
    }
}

impl PartialOrd for HlcTimestamp {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// Hybrid logical clock for one device.
#[derive(Debug)]
pub struct Hlc {
    last: HlcTimestamp,
}

impl Hlc {
    pub fn new(node: u64) -> Self {
        Self {
            last: HlcTimestamp {
                wall_ms: 0,
                counter: 0,
                node,
            },
        }
    }

    /// Timestamp for a local event; always greater than every earlier one.
    pub fn tick(&mut self, clock: &dyn Clock) -> HlcTimestamp {
        let now = clock.now_ms();
        if now > self.last.wall_ms {
            self.last.wall_ms = now;
            self.last.counter = 0;
        } else {
            self.last.counter = self.last.counter.saturating_add(1);
        }
        self.last
    }

    /// Merges a timestamp received from another device; the result is
    /// greater than both, so effects never appear to precede their causes.
    pub fn observe(&mut self, remote: HlcTimestamp, clock: &dyn Clock) -> HlcTimestamp {
        let now = clock.now_ms();
        let wall = now.max(self.last.wall_ms).max(remote.wall_ms);
        let counter = if wall == self.last.wall_ms && wall == remote.wall_ms {
            self.last.counter.max(remote.counter).saturating_add(1)
        } else if wall == self.last.wall_ms {
            self.last.counter.saturating_add(1)
        } else if wall == remote.wall_ms {
            remote.counter.saturating_add(1)
        } else {
            0
        };
        self.last = HlcTimestamp {
            wall_ms: wall,
            counter,
            node: self.last.node,
        };
        self.last
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monotonic_even_when_wall_clock_goes_back() {
        let clock = FixedClock::new(1_000);
        let mut h = Hlc::new(1);
        let a = h.tick(&clock);
        clock.set(500); // device clock jumps backwards
        let b = h.tick(&clock);
        assert!(b > a);
    }

    #[test]
    fn remote_events_order_causally_despite_skew() {
        // Device 2's clock is 10 minutes ahead.
        let c1 = FixedClock::new(1_000);
        let c2 = FixedClock::new(601_000);
        let mut d1 = Hlc::new(1);
        let mut d2 = Hlc::new(2);
        let sent = d2.tick(&c2);
        let received = d1.observe(sent, &c1);
        assert!(received > sent);
        let next = d1.tick(&c1);
        assert!(next > received);
    }
}
