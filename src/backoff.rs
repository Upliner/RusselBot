//! Doubling retry delay with a lower and an upper bound.

use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Backoff {
    first: Duration,
    failures: u32,
}

impl Backoff {
    pub const fn new(first: Duration) -> Backoff {
        Backoff { first, failures: 0 }
    }

    /// The delay to wait now: `first`, doubled for every earlier call since
    /// the last reset, capped at `max`.
    pub fn next_delay(&mut self, max: Duration) -> Duration {
        let factor = 1u32 << self.failures.min(20);
        self.failures += 1;
        self.first.saturating_mul(factor).min(max)
    }

    pub fn reset(&mut self) {
        self.failures = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sequence(first: u64, max: u64, count: usize) -> Vec<u64> {
        let mut backoff = Backoff::new(Duration::from_secs(first));
        (0..count).map(|_| backoff.next_delay(Duration::from_secs(max)).as_secs()).collect()
    }

    #[test]
    fn login_and_teleport_bounds() {
        assert_eq!(sequence(10, 300, 8), [10, 20, 40, 80, 160, 300, 300, 300]);
    }

    #[test]
    fn sit_bounds() {
        assert_eq!(sequence(5, 60, 7), [5, 10, 20, 40, 60, 60, 60]);
    }

    #[test]
    fn refused_login_bound() {
        assert_eq!(sequence(10, 1800, 10), [10, 20, 40, 80, 160, 320, 640, 1280, 1800, 1800]);
    }

    #[test]
    fn reset_returns_to_the_first_delay() {
        let max = Duration::from_secs(300);
        let mut backoff = Backoff::new(Duration::from_secs(10));
        backoff.next_delay(max);
        backoff.next_delay(max);
        backoff.reset();
        assert_eq!(backoff.next_delay(max), Duration::from_secs(10));
    }

    #[test]
    fn never_overflows() {
        let mut backoff = Backoff::new(Duration::from_secs(10));
        for _ in 0..100 {
            assert!(backoff.next_delay(Duration::from_secs(300)) <= Duration::from_secs(300));
        }
    }
}
