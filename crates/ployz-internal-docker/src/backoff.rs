use std::time::{Duration, Instant};

use backon::BackoffBuilder;

#[derive(Clone, Debug)]
pub(crate) struct DockerBackoffBuilder {
    initial: Duration,
    randomization: f64,
    multiplier: f64,
    maximum: Duration,
    maximum_elapsed: Option<Duration>,
}

impl DockerBackoffBuilder {
    pub(crate) fn daemon_readiness() -> Self {
        Self {
            initial: Duration::from_millis(100),
            randomization: 0.5,
            multiplier: 1.5,
            maximum: Duration::from_secs(1),
            maximum_elapsed: None,
        }
    }

    #[cfg(test)]
    fn deterministic(initial: Duration, maximum: Duration) -> Self {
        Self {
            initial,
            randomization: 0.0,
            multiplier: 1.5,
            maximum,
            maximum_elapsed: None,
        }
    }
}

impl BackoffBuilder for DockerBackoffBuilder {
    type Backoff = DockerBackoff;

    fn build(self) -> Self::Backoff {
        DockerBackoff {
            current: self.initial,
            randomization: self.randomization,
            multiplier: self.multiplier,
            maximum: self.maximum,
            maximum_elapsed: self.maximum_elapsed,
            started: Instant::now(),
            rng: fastrand::Rng::new(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct DockerBackoff {
    current: Duration,
    randomization: f64,
    multiplier: f64,
    maximum: Duration,
    maximum_elapsed: Option<Duration>,
    started: Instant,
    rng: fastrand::Rng,
}

impl Iterator for DockerBackoff {
    type Item = Duration;

    fn next(&mut self) -> Option<Self::Item> {
        let delay = randomized(self.current, self.randomization, self.rng.f64());
        if self
            .maximum_elapsed
            .is_some_and(|limit| self.started.elapsed().saturating_add(delay) > limit)
        {
            return None;
        }

        let maximum_nanos = self.maximum.as_nanos();
        let current_nanos = self.current.as_nanos();
        let next_nanos = if current_nanos as f64 >= maximum_nanos as f64 / self.multiplier {
            maximum_nanos
        } else {
            ((current_nanos as f64) * self.multiplier) as u128
        };
        self.current = duration_from_nanos(next_nanos.min(maximum_nanos));
        Some(delay)
    }
}

fn randomized(interval: Duration, factor: f64, random: f64) -> Duration {
    if factor == 0.0 {
        return interval;
    }
    debug_assert!((0.0..=1.0).contains(&random));
    let nanos = interval.as_nanos() as f64;
    let minimum = nanos * (1.0 - factor);
    let maximum = nanos * (1.0 + factor);
    duration_from_nanos((minimum + random * (maximum - minimum + 1.0)) as u128)
}

fn duration_from_nanos(nanos: u128) -> Duration {
    Duration::new(
        (nanos / 1_000_000_000).min(u64::MAX as u128) as u64,
        (nanos % 1_000_000_000) as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_policy_grows_and_caps_the_base_interval() {
        let mut policy =
            DockerBackoffBuilder::deterministic(Duration::from_millis(100), Duration::from_secs(1))
                .build();

        let observed: Vec<_> = policy.by_ref().take(9).collect();
        assert_eq!(
            observed,
            [
                Duration::from_millis(100),
                Duration::from_millis(150),
                Duration::from_millis(225),
                Duration::from_micros(337_500),
                Duration::from_micros(506_250),
                Duration::from_micros(759_375),
                Duration::from_secs(1),
                Duration::from_secs(1),
                Duration::from_secs(1),
            ]
        );
    }

    #[test]
    fn randomized_interval_uses_the_oracle_inclusive_nanosecond_formula() {
        let interval = Duration::from_nanos(100);
        assert_eq!(randomized(interval, 0.5, 0.0), Duration::from_nanos(50));
        assert_eq!(randomized(interval, 0.5, 1.0), Duration::from_nanos(151));
    }
}
