use std::{num::NonZeroUsize, time::Duration};

use ladoflow_core::{
    LatencyAggregator, QualityPolicy, QualityTier, ReconnectDecision, ReconnectPolicy,
    SequenceDisposition, Session, SessionPhase, StreamContinuity, negotiate,
};
use ladoflow_protocol::{Capabilities, CodecSet, FeatureFlags, Hello, InputCapabilities, Role};

fn host_hello() -> Hello {
    Hello::new(1, 3, Role::Host, [0x11; 16], "integration host").expect("valid hello")
}

fn display_hello() -> Hello {
    Hello::new(2, 4, Role::Display, [0x22; 16], "integration display").expect("valid hello")
}

fn host_capabilities() -> Capabilities {
    Capabilities::new(
        3840,
        2160,
        120_000,
        40_000,
        CodecSet::H264 | CodecSet::HEVC,
        InputCapabilities::POINTER | InputCapabilities::TOUCH,
        FeatureFlags::DYNAMIC_ROTATION | FeatureFlags::REMOTE_CURSOR,
    )
    .expect("valid capabilities")
}

fn display_capabilities() -> Capabilities {
    Capabilities::new(
        2560,
        1600,
        90_000,
        24_000,
        CodecSet::H264 | CodecSet::AV1,
        InputCapabilities::TOUCH | InputCapabilities::KEYBOARD,
        FeatureFlags::DYNAMIC_ROTATION | FeatureFlags::AUDIO,
    )
    .expect("valid capabilities")
}

#[test]
fn negotiation_reconnect_telemetry_and_quality_form_one_flow() {
    let agreement = negotiate(
        &host_hello(),
        host_capabilities(),
        &display_hello(),
        display_capabilities(),
    )
    .expect("compatible endpoints");
    assert_eq!(agreement.protocol_version(), 3);
    assert_eq!(agreement.capabilities().max_width(), 2560);
    assert_eq!(agreement.capabilities().max_bitrate_kbps(), 24_000);

    let mut session = Session::new();
    session.start().expect("start session");
    session
        .establish(agreement, StreamContinuity::Restart)
        .expect("establish session");
    assert_eq!(session.phase(), SessionPhase::Active);
    assert_eq!(
        session.observe_sequence(100),
        Ok(SequenceDisposition::Accepted { skipped: 0 })
    );

    let mut latency = LatencyAggregator::new(NonZeroUsize::new(5).expect("non-zero"));
    for millis in [18_u64, 20, 21, 22, 24] {
        latency
            .record(Duration::from_millis(millis))
            .expect("representable sample");
    }
    let snapshot = latency.snapshot().expect("latency snapshot");
    let recommendation =
        QualityPolicy::default().recommend(agreement.capabilities(), Some(&snapshot));
    assert_eq!(recommendation.tier(), QualityTier::High);
    assert_eq!(recommendation.width(), 2560);
    assert_eq!(recommendation.bitrate_kbps(), 24_000);

    session.transport_lost().expect("record transport loss");
    let retry = session
        .schedule_reconnect(
            ReconnectPolicy::new(3, Duration::from_millis(50), Duration::from_secs(1))
                .expect("valid reconnect policy"),
        )
        .expect("schedule reconnect");
    assert_eq!(
        retry,
        ReconnectDecision::RetryAfter {
            attempt: 1,
            delay: Duration::from_millis(50),
            resume_after: Some(100),
        }
    );

    session.begin_reconnect().expect("begin reconnect");
    session
        .establish(agreement, StreamContinuity::Resume)
        .expect("resume stream");
    assert_eq!(session.connection_generation(), 2);
    assert_eq!(session.highest_sequence(), Some(100));
    assert_eq!(
        session.observe_sequence(101),
        Ok(SequenceDisposition::Accepted { skipped: 0 })
    );
}
