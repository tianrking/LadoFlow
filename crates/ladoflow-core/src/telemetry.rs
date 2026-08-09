use std::{collections::VecDeque, fmt, num::NonZeroUsize, time::Duration};

/// Error raised while adding a latency measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelemetryError {
    /// The duration cannot be represented at the aggregator's microsecond precision.
    SampleTooLarge,
}

impl fmt::Display for TelemetryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("latency sample exceeds microsecond representation")
    }
}

impl std::error::Error for TelemetryError {}

/// Immutable summary of the current rolling latency window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LatencySnapshot {
    sample_count: usize,
    total_sample_count: u64,
    min: Duration,
    mean: Duration,
    p50: Duration,
    p95: Duration,
    max: Duration,
    jitter: Duration,
}

impl LatencySnapshot {
    /// Number of samples represented by this rolling snapshot.
    #[must_use]
    pub const fn sample_count(self) -> usize {
        self.sample_count
    }

    /// Number of samples observed by the aggregator, including evicted values.
    #[must_use]
    pub const fn total_sample_count(self) -> u64 {
        self.total_sample_count
    }

    /// Lowest latency in the current window.
    #[must_use]
    pub const fn min(self) -> Duration {
        self.min
    }

    /// Arithmetic mean latency in the current window.
    #[must_use]
    pub const fn mean(self) -> Duration {
        self.mean
    }

    /// Nearest-rank 50th-percentile latency in the current window.
    #[must_use]
    pub const fn p50(self) -> Duration {
        self.p50
    }

    /// Nearest-rank 95th-percentile latency in the current window.
    #[must_use]
    pub const fn p95(self) -> Duration {
        self.p95
    }

    /// Highest latency in the current window.
    #[must_use]
    pub const fn max(self) -> Duration {
        self.max
    }

    /// Mean absolute difference between adjacent samples in arrival order.
    #[must_use]
    pub const fn jitter(self) -> Duration {
        self.jitter
    }
}

/// Bounded rolling latency collector with deterministic summary statistics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatencyAggregator {
    samples_micros: VecDeque<u64>,
    capacity: NonZeroUsize,
    total_sample_count: u64,
}

impl LatencyAggregator {
    /// Construct an empty collector retaining at most `capacity` samples.
    #[must_use]
    pub fn new(capacity: NonZeroUsize) -> Self {
        Self {
            samples_micros: VecDeque::with_capacity(capacity.get()),
            capacity,
            total_sample_count: 0,
        }
    }

    /// Number of samples currently retained.
    #[must_use]
    pub fn len(&self) -> usize {
        self.samples_micros.len()
    }

    /// Whether the rolling window currently contains no samples.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.samples_micros.is_empty()
    }

    /// Maximum number of samples retained at once.
    #[must_use]
    pub const fn capacity(&self) -> NonZeroUsize {
        self.capacity
    }

    /// Number of samples observed, including those evicted from the window.
    #[must_use]
    pub const fn total_sample_count(&self) -> u64 {
        self.total_sample_count
    }

    /// Add one latency measurement, evicting the oldest sample when full.
    ///
    /// Samples are stored with microsecond precision. Sub-microsecond portions
    /// are truncated consistently with [`Duration::as_micros`].
    ///
    /// # Errors
    ///
    /// Returns [`TelemetryError::SampleTooLarge`] when the duration contains
    /// more microseconds than can be represented by `u64`.
    pub fn record(&mut self, latency: Duration) -> Result<(), TelemetryError> {
        let micros =
            u64::try_from(latency.as_micros()).map_err(|_| TelemetryError::SampleTooLarge)?;

        if self.samples_micros.len() == self.capacity.get() {
            self.samples_micros.pop_front();
        }
        self.samples_micros.push_back(micros);
        self.total_sample_count = self.total_sample_count.saturating_add(1);
        Ok(())
    }

    /// Summarize the retained window, or return `None` while it is empty.
    #[must_use]
    pub fn snapshot(&self) -> Option<LatencySnapshot> {
        if self.samples_micros.is_empty() {
            return None;
        }

        let mut sorted = self.samples_micros.iter().copied().collect::<Vec<_>>();
        sorted.sort_unstable();

        let sample_count = sorted.len();
        let sample_count_u128 = u128::try_from(sample_count).ok()?;
        let sum = sorted
            .iter()
            .map(|&sample| u128::from(sample))
            .sum::<u128>();
        let mean_micros = u64::try_from(sum / sample_count_u128).ok()?;
        let jitter_micros = mean_adjacent_difference(&self.samples_micros);
        let min_micros = sorted.first().copied()?;
        let max_micros = sorted.last().copied()?;

        Some(LatencySnapshot {
            sample_count,
            total_sample_count: self.total_sample_count,
            min: Duration::from_micros(min_micros),
            mean: Duration::from_micros(mean_micros),
            p50: Duration::from_micros(nearest_rank(&sorted, 50)),
            p95: Duration::from_micros(nearest_rank(&sorted, 95)),
            max: Duration::from_micros(max_micros),
            jitter: Duration::from_micros(jitter_micros),
        })
    }
}

fn nearest_rank(sorted: &[u64], percentile: usize) -> u64 {
    debug_assert!(!sorted.is_empty());
    debug_assert!((1..=100).contains(&percentile));

    let quotient = sorted.len() / 100;
    let remainder = sorted.len() % 100;
    let rank = quotient * percentile + (remainder * percentile).div_ceil(100);
    sorted.get(rank.max(1) - 1).copied().unwrap_or_default()
}

fn mean_adjacent_difference(samples: &VecDeque<u64>) -> u64 {
    if samples.len() < 2 {
        return 0;
    }

    let mut iterator = samples.iter().copied();
    let Some(mut previous) = iterator.next() else {
        return 0;
    };
    let mut difference_sum = 0_u128;
    for current in iterator {
        difference_sum += u128::from(current.abs_diff(previous));
        previous = current;
    }

    let Ok(difference_count) = u128::try_from(samples.len() - 1) else {
        return 0;
    };
    u64::try_from(difference_sum / difference_count).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::{num::NonZeroUsize, time::Duration};

    use super::{LatencyAggregator, TelemetryError};

    #[test]
    fn empty_aggregator_has_no_snapshot() {
        let aggregator = LatencyAggregator::new(NonZeroUsize::new(3).expect("non-zero"));

        assert!(aggregator.is_empty());
        assert_eq!(aggregator.snapshot(), None);
        assert_eq!(aggregator.total_sample_count(), 0);
    }

    #[test]
    fn computes_rolling_statistics_and_evicts_oldest_sample() {
        let mut aggregator = LatencyAggregator::new(NonZeroUsize::new(4).expect("non-zero"));
        for millis in [10_u64, 20, 40, 80, 100] {
            aggregator
                .record(Duration::from_millis(millis))
                .expect("representable sample");
        }

        let snapshot = aggregator.snapshot().expect("non-empty snapshot");
        assert_eq!(snapshot.sample_count(), 4);
        assert_eq!(snapshot.total_sample_count(), 5);
        assert_eq!(snapshot.min(), Duration::from_millis(20));
        assert_eq!(snapshot.mean(), Duration::from_millis(60));
        assert_eq!(snapshot.p50(), Duration::from_millis(40));
        assert_eq!(snapshot.p95(), Duration::from_millis(100));
        assert_eq!(snapshot.max(), Duration::from_millis(100));
        assert_eq!(snapshot.jitter(), Duration::from_micros(80_000 / 3));
    }

    #[test]
    fn single_sample_has_zero_jitter_and_microsecond_precision() {
        let mut aggregator = LatencyAggregator::new(NonZeroUsize::new(1).expect("non-zero"));
        aggregator
            .record(Duration::from_nanos(1_999))
            .expect("representable sample");

        let snapshot = aggregator.snapshot().expect("non-empty snapshot");
        assert_eq!(snapshot.mean(), Duration::from_micros(1));
        assert_eq!(snapshot.jitter(), Duration::ZERO);
    }

    #[test]
    fn rejects_duration_that_exceeds_microsecond_storage() {
        let mut aggregator = LatencyAggregator::new(NonZeroUsize::new(1).expect("non-zero"));

        assert_eq!(
            aggregator.record(Duration::new(u64::MAX, 999_999_999)),
            Err(TelemetryError::SampleTooLarge)
        );
        assert!(aggregator.is_empty());
    }
}
