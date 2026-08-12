use std::time::{Duration, Instant};

use backon::BackoffBuilder;

/// The package-local implementation of cenkalti/backoff's interval policy.
#[derive(Clone, Debug)]
pub(crate) struct RandomizedBackoff {
    initial: Duration,
    factor: f64,
    multiplier: f64,
    maximum: Duration,
    max_elapsed: Option<Duration>,
}

impl RandomizedBackoff {
    pub(crate) fn transport() -> Self {
        Self::new(
            Duration::from_millis(100),
            Duration::from_secs(60),
            Some(Duration::from_secs(2)),
        )
    }

    pub(crate) fn subscription() -> Self {
        Self::new(
            Duration::from_millis(100),
            Duration::from_secs(1),
            Some(Duration::from_secs(60)),
        )
    }

    pub(crate) const fn new(
        initial: Duration,
        maximum: Duration,
        max_elapsed: Option<Duration>,
    ) -> Self {
        Self {
            initial,
            factor: 0.5,
            multiplier: 1.5,
            maximum,
            max_elapsed,
        }
    }

    #[cfg(test)]
    pub(crate) const fn deterministic(
        initial: Duration,
        maximum: Duration,
        max_elapsed: Option<Duration>,
    ) -> Self {
        Self {
            initial,
            factor: 0.0,
            multiplier: 1.5,
            maximum,
            max_elapsed,
        }
    }
}

impl BackoffBuilder for RandomizedBackoff {
    type Backoff = RandomizedBackoffIter;

    fn build(self) -> Self::Backoff {
        RandomizedBackoffIter {
            current: self.initial,
            factor: self.factor,
            multiplier: self.multiplier,
            maximum: self.maximum,
            max_elapsed: self.max_elapsed,
            started: Instant::now(),
        }
    }
}

pub(crate) struct RandomizedBackoffIter {
    current: Duration,
    factor: f64,
    multiplier: f64,
    maximum: Duration,
    max_elapsed: Option<Duration>,
    started: Instant,
}

impl Iterator for RandomizedBackoffIter {
    type Item = Duration;

    fn next(&mut self) -> Option<Self::Item> {
        let interval = if self.factor == 0.0 {
            self.current
        } else {
            randomized(self.current, self.factor, fastrand::f64())
        };

        if let Some(limit) = self.max_elapsed
            && self.started.elapsed().saturating_add(interval) > limit
        {
            return None;
        }

        let current_nanos = self.current.as_nanos();
        let maximum_nanos = self.maximum.as_nanos();
        let next_nanos = if (current_nanos as f64) >= (maximum_nanos as f64 / self.multiplier) {
            maximum_nanos
        } else {
            ((current_nanos as f64) * self.multiplier) as u128
        };
        self.current = duration_from_nanos(next_nanos.min(maximum_nanos));
        Some(interval)
    }
}

fn randomized(interval: Duration, factor: f64, random: f64) -> Duration {
    debug_assert!((0.0..=1.0).contains(&random));
    let nanos = interval.as_nanos() as f64;
    let minimum = nanos * (1.0 - factor);
    let maximum = nanos * (1.0 + factor);
    duration_from_nanos((minimum + random * (maximum - minimum + 1.0)) as u128)
}

fn duration_from_nanos(nanos: u128) -> Duration {
    let secs = (nanos / 1_000_000_000).min(u64::MAX as u128) as u64;
    let subsec = if secs == u64::MAX {
        999_999_999
    } else {
        (nanos % 1_000_000_000) as u32
    };
    Duration::new(secs, subsec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_policy_grows_and_caps() {
        let builder = RandomizedBackoff::deterministic(
            Duration::from_millis(100),
            Duration::from_secs(1),
            None,
        );
        let got: Vec<_> = builder.build().take(8).collect();
        assert_eq!(
            got,
            [
                Duration::from_millis(100),
                Duration::from_millis(150),
                Duration::from_millis(225),
                Duration::from_micros(337_500),
                Duration::from_micros(506_250),
                Duration::from_micros(759_375),
                Duration::from_secs(1),
                Duration::from_secs(1),
            ]
        );
    }

    #[test]
    fn injected_randomization_matches_interval_edges() {
        let interval = Duration::from_millis(100);
        assert_eq!(randomized(interval, 0.5, 0.0), Duration::from_millis(50));
        assert_eq!(
            randomized(interval, 0.5, 1.0),
            Duration::from_nanos(150_000_001)
        );
    }
}
