use std::{fmt, time::Duration};

use crate::{LatencySnapshot, NegotiatedCapabilities};

/// Coarse quality level selected by [`QualityPolicy`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityTier {
    /// Use every negotiated scalar limit.
    High,
    /// Use three-quarter resolution, at most 60 Hz, and 70% bitrate.
    Balanced,
    /// Use half resolution, at most 30 Hz, and 40% bitrate.
    Constrained,
}

/// Concrete stream limits recommended for the next encoder configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QualityRecommendation {
    tier: QualityTier,
    width: u16,
    height: u16,
    refresh_millihz: u32,
    bitrate_kbps: u32,
}

impl QualityRecommendation {
    /// Coarse policy tier behind this recommendation.
    #[must_use]
    pub const fn tier(self) -> QualityTier {
        self.tier
    }

    /// Recommended coded width in pixels.
    #[must_use]
    pub const fn width(self) -> u16 {
        self.width
    }

    /// Recommended coded height in pixels.
    #[must_use]
    pub const fn height(self) -> u16 {
        self.height
    }

    /// Recommended refresh rate in millihertz.
    #[must_use]
    pub const fn refresh_millihz(self) -> u32 {
        self.refresh_millihz
    }

    /// Recommended bitrate in kilobits per second.
    #[must_use]
    pub const fn bitrate_kbps(self) -> u32 {
        self.bitrate_kbps
    }
}

/// Stateless thresholds for converting latency telemetry into stream limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QualityPolicy {
    high_p95: Duration,
    high_jitter: Duration,
    constrained_p95: Duration,
    constrained_jitter: Duration,
}

impl QualityPolicy {
    /// Construct policy thresholds.
    ///
    /// High quality requires both measurements to be at or below the high
    /// thresholds. Constrained quality is selected when either measurement is
    /// at or above its constrained threshold. Other samples are balanced.
    ///
    /// # Errors
    ///
    /// Returns [`QualityPolicyError`] unless each high threshold is strictly
    /// lower than its corresponding constrained threshold.
    pub fn new(
        high_p95: Duration,
        high_jitter: Duration,
        constrained_p95: Duration,
        constrained_jitter: Duration,
    ) -> Result<Self, QualityPolicyError> {
        if high_p95 >= constrained_p95 || high_jitter >= constrained_jitter {
            Err(QualityPolicyError::ThresholdsOutOfOrder)
        } else {
            Ok(Self {
                high_p95,
                high_jitter,
                constrained_p95,
                constrained_jitter,
            })
        }
    }

    /// Recommend stream limits within the negotiated capability maxima.
    ///
    /// A missing snapshot selects balanced quality, avoiding an optimistic
    /// startup spike before any latency evidence exists.
    #[must_use]
    pub fn recommend(
        self,
        capabilities: NegotiatedCapabilities,
        snapshot: Option<&LatencySnapshot>,
    ) -> QualityRecommendation {
        let tier = snapshot.map_or(QualityTier::Balanced, |snapshot| {
            if snapshot.p95() <= self.high_p95 && snapshot.jitter() <= self.high_jitter {
                QualityTier::High
            } else if snapshot.p95() >= self.constrained_p95
                || snapshot.jitter() >= self.constrained_jitter
            {
                QualityTier::Constrained
            } else {
                QualityTier::Balanced
            }
        });

        let (dimension_percent, bitrate_percent, refresh_cap) = match tier {
            QualityTier::High => (100, 100, u32::MAX),
            QualityTier::Balanced => (75, 70, 60_000),
            QualityTier::Constrained => (50, 40, 30_000),
        };

        QualityRecommendation {
            tier,
            width: scale_u16_percent(capabilities.max_width(), dimension_percent),
            height: scale_u16_percent(capabilities.max_height(), dimension_percent),
            refresh_millihz: capabilities.max_refresh_millihz().min(refresh_cap),
            bitrate_kbps: scale_u32_percent(capabilities.max_bitrate_kbps(), bitrate_percent),
        }
    }
}

impl Default for QualityPolicy {
    fn default() -> Self {
        Self {
            high_p95: Duration::from_millis(40),
            high_jitter: Duration::from_millis(8),
            constrained_p95: Duration::from_millis(90),
            constrained_jitter: Duration::from_millis(20),
        }
    }
}

/// Invalid quality-policy configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityPolicyError {
    /// A high-quality threshold is not below its constrained counterpart.
    ThresholdsOutOfOrder,
}

impl fmt::Display for QualityPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("quality thresholds are not strictly ordered")
    }
}

impl std::error::Error for QualityPolicyError {}

fn scale_u16_percent(value: u16, percent: u16) -> u16 {
    debug_assert!(percent <= 100);
    let whole = (value / 100) * percent;
    let remainder = ((value % 100) * percent) / 100;
    (whole + remainder).max(1)
}

fn scale_u32_percent(value: u32, percent: u32) -> u32 {
    debug_assert!(percent <= 100);
    let whole = (value / 100) * percent;
    let remainder = ((value % 100) * percent) / 100;
    (whole + remainder).max(1)
}

#[cfg(test)]
mod tests {
    use std::{num::NonZeroUsize, time::Duration};

    use ladoflow_protocol::{Capabilities, CodecSet, FeatureFlags, Hello, InputCapabilities, Role};

    use crate::{LatencyAggregator, NegotiatedCapabilities, negotiate};

    use super::{QualityPolicy, QualityPolicyError, QualityTier};

    fn capabilities_with_limits(
        width: u16,
        height: u16,
        refresh_millihz: u32,
        bitrate_kbps: u32,
    ) -> NegotiatedCapabilities {
        let host = Hello::new(1, 1, Role::Host, [1; 16], "test host").expect("valid hello");
        let display =
            Hello::new(1, 1, Role::Display, [2; 16], "test display").expect("valid hello");
        let capabilities = Capabilities::new(
            width,
            height,
            refresh_millihz,
            bitrate_kbps,
            CodecSet::H264,
            InputCapabilities::default(),
            FeatureFlags::default(),
        )
        .expect("valid capabilities");

        negotiate(&host, capabilities, &display, capabilities)
            .expect("compatible endpoints")
            .capabilities()
    }

    fn capabilities() -> NegotiatedCapabilities {
        capabilities_with_limits(2000, 1200, 120_000, 10_000)
    }

    fn snapshot(latencies: &[u64]) -> crate::LatencySnapshot {
        let mut aggregator =
            LatencyAggregator::new(NonZeroUsize::new(latencies.len()).expect("non-empty samples"));
        for &millis in latencies {
            aggregator
                .record(Duration::from_millis(millis))
                .expect("representable sample");
        }
        aggregator.snapshot().expect("non-empty snapshot")
    }

    #[test]
    fn starts_balanced_without_latency_evidence() {
        let recommendation = QualityPolicy::default().recommend(capabilities(), None);

        assert_eq!(recommendation.tier(), QualityTier::Balanced);
        assert_eq!(recommendation.width(), 1500);
        assert_eq!(recommendation.height(), 900);
        assert_eq!(recommendation.refresh_millihz(), 60_000);
        assert_eq!(recommendation.bitrate_kbps(), 7_000);
    }

    #[test]
    fn selects_high_for_stable_low_latency() {
        let telemetry = snapshot(&[20, 22, 24, 23]);
        let recommendation = QualityPolicy::default().recommend(capabilities(), Some(&telemetry));

        assert_eq!(recommendation.tier(), QualityTier::High);
        assert_eq!(recommendation.width(), 2000);
        assert_eq!(recommendation.height(), 1200);
        assert_eq!(recommendation.refresh_millihz(), 120_000);
        assert_eq!(recommendation.bitrate_kbps(), 10_000);
    }

    #[test]
    fn selects_constrained_for_high_tail_latency_or_jitter() {
        let high_tail = snapshot(&[30, 35, 40, 95]);
        let high_jitter = snapshot(&[30, 55, 30, 55]);

        for telemetry in [high_tail, high_jitter] {
            let recommendation =
                QualityPolicy::default().recommend(capabilities(), Some(&telemetry));
            assert_eq!(recommendation.tier(), QualityTier::Constrained);
            assert_eq!(recommendation.width(), 1000);
            assert_eq!(recommendation.height(), 600);
            assert_eq!(recommendation.refresh_millihz(), 30_000);
            assert_eq!(recommendation.bitrate_kbps(), 4_000);
        }
    }

    #[test]
    fn rejects_unordered_thresholds() {
        assert_eq!(
            QualityPolicy::new(
                Duration::from_millis(90),
                Duration::from_millis(5),
                Duration::from_millis(90),
                Duration::from_millis(20),
            ),
            Err(QualityPolicyError::ThresholdsOutOfOrder)
        );
    }

    #[test]
    fn scaling_never_exceeds_capabilities_or_reaches_zero() {
        let tiny = capabilities_with_limits(1, 1, 1, 1);
        let recommendation = QualityPolicy::default().recommend(tiny, None);

        assert_eq!(recommendation.width(), 1);
        assert_eq!(recommendation.height(), 1);
        assert_eq!(recommendation.refresh_millihz(), 1);
        assert_eq!(recommendation.bitrate_kbps(), 1);
    }
}
