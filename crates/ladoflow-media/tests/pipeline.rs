use std::time::Duration;

use ladoflow_media::{
    FrameDimensions, FrameKind, FramePacer, FrameRate, IdleReason, LatestFrameScheduler,
    PaceDecision, PollOutcome, SchedulerConfig, SubmitOutcome, SyntheticConfig,
    SyntheticFrameProducer, VideoFormat,
};

fn format_at(rate: FrameRate) -> VideoFormat {
    VideoFormat::new(
        FrameDimensions::new(1920, 1080).expect("valid test dimensions"),
        rate,
    )
}

fn producer(rate: FrameRate, keyframe_interval: u64) -> SyntheticFrameProducer {
    let config = SyntheticConfig::new(format_at(rate), 64, keyframe_interval)
        .expect("valid synthetic configuration")
        .with_seed(0x1234_5678_9abc_def0);
    SyntheticFrameProducer::new(config)
}

fn expect_due(decision: PaceDecision) -> ladoflow_media::PacingTick {
    match decision {
        PaceDecision::Due(tick) => tick,
        PaceDecision::WaitUntil(deadline) => {
            panic!("expected due tick, waiting until {deadline:?}")
        }
        PaceDecision::Exhausted => panic!("test pacer unexpectedly exhausted"),
    }
}

fn verify_pacing(rate: FrameRate, final_tick: u64) {
    let origin = Duration::from_secs(7);
    let mut pacer = FramePacer::new(rate, origin);

    for index in 0..=final_tick {
        let deadline = origin + rate.timestamp(index);
        if deadline > origin {
            let just_before = deadline
                .checked_sub(Duration::from_nanos(1))
                .expect("non-zero test deadline");
            assert_eq!(pacer.poll(just_before), PaceDecision::WaitUntil(deadline));
        }

        let tick = expect_due(pacer.poll(deadline));
        assert_eq!(tick.index(), index);
        assert_eq!(tick.deadline(), deadline);
        assert_eq!(tick.skipped(), 0);
    }
}

#[test]
fn pacer_holds_exact_30_hz_deadlines_without_drift() {
    let rate = FrameRate::from_hz(30).expect("valid rate");
    verify_pacing(rate, 300);

    assert_eq!(rate.timestamp(1), Duration::from_nanos(33_333_333));
    assert_eq!(rate.timestamp(3), Duration::from_millis(100));
    assert_eq!(rate.timestamp(300), Duration::from_secs(10));
}

#[test]
fn pacer_holds_exact_60_hz_deadlines_without_drift() {
    let rate = FrameRate::from_hz(60).expect("valid rate");
    verify_pacing(rate, 600);

    assert_eq!(rate.timestamp(1), Duration::from_nanos(16_666_666));
    assert_eq!(rate.timestamp(3), Duration::from_millis(50));
    assert_eq!(rate.timestamp(600), Duration::from_secs(10));
}

#[test]
fn delayed_pacer_poll_skips_catch_up_bursts() {
    let rate = FrameRate::from_hz(60).expect("valid rate");
    let mut pacer = FramePacer::new(rate, Duration::ZERO);
    assert_eq!(expect_due(pacer.poll(Duration::ZERO)).index(), 0);

    let tick = expect_due(pacer.poll(Duration::from_millis(100)));
    assert_eq!(tick.index(), 6);
    assert_eq!(tick.deadline(), Duration::from_millis(100));
    assert_eq!(tick.skipped(), 5);
    assert_eq!(
        pacer.poll(Duration::from_millis(100)),
        PaceDecision::WaitUntil(rate.timestamp(7))
    );
}

#[test]
fn synthetic_frames_have_stable_timestamps_and_keyframe_cadence() {
    let rate = FrameRate::from_hz(60).expect("valid rate");
    let frames: Vec<_> = producer(rate, 30).take(61).collect();

    let keyframes: Vec<_> = frames
        .iter()
        .filter(|frame| frame.metadata().kind() == FrameKind::Key)
        .map(|frame| frame.metadata().sequence())
        .collect();
    assert_eq!(keyframes, vec![0, 30, 60]);

    assert_eq!(frames[0].metadata().presentation_time(), Duration::ZERO);
    assert_eq!(
        frames[1].metadata().presentation_time(),
        Duration::from_nanos(16_666_666)
    );
    assert_eq!(
        frames[2].metadata().presentation_time(),
        Duration::from_nanos(33_333_333)
    );
    assert_eq!(
        frames[1].metadata().duration(),
        Duration::from_nanos(16_666_667)
    );
    assert_eq!(
        frames[60].metadata().presentation_time(),
        Duration::from_secs(1)
    );
    assert!(
        frames.iter().all(|frame| {
            frame.metadata().capture_time() == frame.metadata().presentation_time()
        })
    );
}

#[test]
fn synthetic_payloads_are_repeatable_and_sequence_specific() {
    let rate = FrameRate::from_hz(30).expect("valid rate");
    let mut first = producer(rate, 30);
    let mut second = producer(rate, 30);

    let first_zero = first.next_frame().expect("frame zero");
    let second_zero = second.next_frame().expect("frame zero");
    let first_one = first.next_frame().expect("frame one");

    assert_eq!(first_zero, second_zero);
    assert_ne!(first_zero.payload(), first_one.payload());
    assert_eq!(first_zero.payload_len(), 64);
}

#[test]
fn latest_frame_replaces_and_drops_superseded_work() {
    let rate = FrameRate::from_hz(30).expect("valid rate");
    let config = SchedulerConfig::new(rate, Duration::from_millis(100), 1024)
        .expect("valid scheduler configuration");
    let mut scheduler = LatestFrameScheduler::new(config, Duration::ZERO);
    let mut frames = producer(rate, 30);
    let frame_zero = frames.next_frame().expect("frame zero");
    let frame_one = frames.next_frame().expect("frame one");

    assert_eq!(
        scheduler.submit(frame_zero.clone(), Duration::ZERO),
        SubmitOutcome::Queued
    );
    assert_eq!(
        scheduler.submit(frame_one, Duration::from_millis(1)),
        SubmitOutcome::Replaced {
            dropped_sequence: 0
        }
    );
    assert_eq!(
        scheduler.submit(frame_zero, Duration::from_millis(2)),
        SubmitOutcome::DroppedSuperseded {
            sequence: 0,
            kept_sequence: 1
        }
    );
    assert_eq!(scheduler.pending_sequence(), Some(1));

    let deadline = rate.timestamp(1);
    let ready = match scheduler.poll(deadline) {
        PollOutcome::Ready(ready) => ready,
        outcome => panic!("expected ready frame, got {outcome:?}"),
    };
    assert_eq!(ready.frame().metadata().sequence(), 1);
    assert_eq!(ready.pacing_tick().index(), 1);
    assert_eq!(ready.pacing_tick().skipped(), 1);
    assert_eq!(
        ready.queue_time(),
        deadline
            .checked_sub(Duration::from_millis(1))
            .expect("frame deadline exceeds enqueue time")
    );
    assert_eq!(ready.pacing_lateness(), Duration::ZERO);

    let metrics = scheduler.metrics();
    assert_eq!(metrics.submitted_frames(), 3);
    assert_eq!(metrics.presented_frames(), 1);
    assert_eq!(metrics.dropped_superseded_frames(), 2);
    assert_eq!(metrics.skipped_pacing_ticks(), 1);
}

#[test]
fn stale_frames_drop_at_submission_and_dispatch() {
    let rate = FrameRate::from_hz(30).expect("valid rate");
    let config = SchedulerConfig::new(rate, Duration::from_millis(10), 1024)
        .expect("valid scheduler configuration");
    let mut scheduler = LatestFrameScheduler::new(config, Duration::ZERO);
    let mut frames = producer(rate, 30);

    let frame_zero = frames.next_frame().expect("frame zero");
    assert_eq!(
        scheduler.submit(frame_zero, Duration::from_millis(11)),
        SubmitOutcome::DroppedStale { sequence: 0 }
    );

    let frame_one = frames.next_frame().expect("frame one");
    let presentation_time = frame_one.metadata().presentation_time();
    assert_eq!(
        scheduler.submit(frame_one, presentation_time),
        SubmitOutcome::Queued
    );
    let dispatch_time = presentation_time + Duration::from_millis(11);
    assert!(matches!(
        scheduler.poll(dispatch_time),
        PollOutcome::DroppedStale { sequence: 1, .. }
    ));

    let metrics = scheduler.metrics();
    assert_eq!(metrics.dropped_stale_frames(), 2);
    assert_eq!(metrics.dropped_frames(), 2);
    assert_eq!(metrics.presented_frames(), 0);
    assert_eq!(metrics.idle_pacing_ticks(), 1);
}

#[test]
fn future_frame_waits_for_its_media_timestamp() {
    let rate = FrameRate::from_hz(30).expect("valid rate");
    let config = SchedulerConfig::new(rate, Duration::from_millis(100), 1024)
        .expect("valid scheduler configuration");
    let mut scheduler = LatestFrameScheduler::new(config, Duration::ZERO);
    let frame_one = producer(rate, 30).nth(1).expect("frame one");
    assert_eq!(
        scheduler.submit(frame_one, Duration::ZERO),
        SubmitOutcome::Queued
    );

    let tick = match scheduler.poll(Duration::ZERO) {
        PollOutcome::Idle { tick, reason } => {
            assert_eq!(reason, IdleReason::FrameNotDue);
            tick
        }
        outcome => panic!("expected an idle pacing tick, got {outcome:?}"),
    };
    assert_eq!(tick.index(), 0);
    assert_eq!(tick.deadline(), Duration::ZERO);

    let deadline = rate.timestamp(1);
    let ready = match scheduler.poll(deadline) {
        PollOutcome::Ready(ready) => ready,
        outcome => panic!("expected frame at its media timestamp, got {outcome:?}"),
    };
    assert_eq!(ready.frame().metadata().sequence(), 1);
}

#[test]
fn scheduler_reports_queue_pacing_and_capture_latency() {
    let rate = FrameRate::from_hz(60).expect("valid rate");
    let config = SchedulerConfig::new(rate, Duration::from_millis(50), 1024)
        .expect("valid scheduler configuration");
    let origin = Duration::from_secs(10);
    let mut scheduler = LatestFrameScheduler::new(config, origin);
    let frame_zero = producer(rate, 60).next_frame().expect("frame zero");

    assert_eq!(
        scheduler.submit(frame_zero, origin + Duration::from_millis(2)),
        SubmitOutcome::Queued
    );
    let ready = match scheduler.poll(origin + Duration::from_millis(5)) {
        PollOutcome::Ready(ready) => ready,
        outcome => panic!("expected ready frame, got {outcome:?}"),
    };
    assert_eq!(ready.queue_time(), Duration::from_millis(3));
    assert_eq!(ready.pacing_lateness(), Duration::from_millis(5));
    assert_eq!(ready.frame_latency(), Duration::from_millis(5));

    let metrics = scheduler.take_metrics();
    assert_eq!(metrics.max_queue_time(), Duration::from_millis(3));
    assert_eq!(metrics.average_queue_time(), Some(Duration::from_millis(3)));
    assert_eq!(metrics.max_pacing_lateness(), Duration::from_millis(5));
    assert_eq!(
        metrics.average_pacing_lateness(),
        Some(Duration::from_millis(5))
    );
    assert_eq!(metrics.max_frame_latency(), Duration::from_millis(5));
    assert_eq!(
        metrics.average_frame_latency(),
        Some(Duration::from_millis(5))
    );
    assert_eq!(scheduler.metrics().submitted_frames(), 0);
    assert_eq!(scheduler.metrics().average_queue_time(), None);
}

#[test]
fn scheduler_rejects_payloads_above_its_memory_bound() {
    let rate = FrameRate::from_hz(30).expect("valid rate");
    let config = SchedulerConfig::new(rate, Duration::from_millis(100), 32)
        .expect("valid scheduler configuration");
    let mut scheduler = LatestFrameScheduler::new(config, Duration::ZERO);
    let frame = producer(rate, 30).next_frame().expect("frame zero");

    assert_eq!(
        scheduler.submit(frame, Duration::ZERO),
        SubmitOutcome::DroppedOversized {
            sequence: 0,
            payload_bytes: 64,
            limit: 32,
        }
    );
    assert!(!scheduler.has_pending_frame());
    assert_eq!(scheduler.metrics().dropped_oversized_frames(), 1);
}
