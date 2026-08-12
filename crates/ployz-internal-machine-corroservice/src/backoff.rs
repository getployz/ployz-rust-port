use std::time::{Duration, Instant};

use backon::BackoffBuilder;

#[derive(Clone, Debug)]
pub(crate) struct ReadinessBackoff {
    initial: Duration,
    maximum: Duration,
    max_elapsed: Duration,
    randomization: f64,
}

impl Default for ReadinessBackoff {
    fn default() -> Self {
        Self {
            initial: Duration::from_millis(50),
            maximum: Duration::from_secs(1),
            max_elapsed: Duration::from_secs(15),
            randomization: 0.5,
        }
    }
}

impl BackoffBuilder for ReadinessBackoff {
    type Backoff = ReadinessBackoffIter;

    fn build(self) -> Self::Backoff {
        ReadinessBackoffIter {
            current: self.initial,
            maximum: self.maximum,
            max_elapsed: self.max_elapsed,
            randomization: self.randomization,
            started: Instant::now(),
        }
    }
}

pub(crate) struct ReadinessBackoffIter {
    current: Duration,
    maximum: Duration,
    max_elapsed: Duration,
    randomization: f64,
    started: Instant,
}

impl Iterator for ReadinessBackoffIter {
    type Item = Duration;

    fn next(&mut self) -> Option<Self::Item> {
        let interval = if self.randomization == 0.0 {
            self.current
        } else {
            randomized(self.current, self.randomization, fastrand::f64())
        };
        if self.started.elapsed().saturating_add(interval) > self.max_elapsed {
            return None;
        }

        let current = self.current.as_nanos();
        let maximum = self.maximum.as_nanos();
        let next = if (current as f64) >= (maximum as f64 / 1.5) {
            maximum
        } else {
            ((current as f64) * 1.5) as u128
        };
        self.current = duration_from_nanos(next.min(maximum));
        Some(interval)
    }
}

fn randomized(interval: Duration, factor: f64, random: f64) -> Duration {
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
    fn readiness_schedule_grows_by_one_and_a_half_and_caps_at_one_second() {
        let builder = ReadinessBackoff {
            max_elapsed: Duration::from_secs(30),
            randomization: 0.0,
            ..ReadinessBackoff::default()
        };
        assert_eq!(
            builder.build().take(10).collect::<Vec<_>>(),
            [
                Duration::from_millis(50),
                Duration::from_millis(75),
                Duration::from_micros(112_500),
                Duration::from_micros(168_750),
                Duration::from_micros(253_125),
                Duration::from_nanos(379_687_500),
                Duration::from_nanos(569_531_250),
                Duration::from_nanos(854_296_875),
                Duration::from_secs(1),
                Duration::from_secs(1),
            ]
        );
    }
}
