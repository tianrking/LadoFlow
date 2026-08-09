use std::{fmt, time::Duration};

use crate::NegotiatedSession;

/// Observable lifecycle phase of a logical display session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPhase {
    /// No connection attempt has started.
    Idle,
    /// Hello and capability exchange is in progress.
    Negotiating,
    /// A transport is active with an agreed configuration.
    Active,
    /// A recoverable loss occurred and no retry is currently scheduled.
    WaitingToReconnect,
    /// A retry delay was selected and must elapse before reconnecting.
    ReconnectScheduled,
    /// The session was closed explicitly or exhausted its retry budget.
    Closed,
}

/// Whether an established transport starts a new stream or resumes the old one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamContinuity {
    /// Start a new sequence space, discarding the previous receive cursor.
    Restart,
    /// Continue the prior sequence space after a transient transport loss.
    Resume,
}

/// Classification of an observed inbound sequence number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceDisposition {
    /// Sequence advanced; `skipped` is the number of missing values before it.
    Accepted {
        /// Count of sequence values skipped since the prior accepted value.
        skipped: u64,
    },
    /// Sequence equals the most recently accepted value.
    Duplicate,
    /// Sequence predates the receive cursor and must not move it backwards.
    Stale {
        /// Highest sequence value accepted by this logical stream.
        highest_accepted: u64,
    },
}

/// Bounded exponential-backoff settings for transient transport loss.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReconnectPolicy {
    max_attempts: u32,
    initial_delay: Duration,
    max_delay: Duration,
}

impl ReconnectPolicy {
    /// Construct a reconnect policy.
    ///
    /// A zero attempt count disables reconnects. Zero delays are permitted for
    /// runtimes that provide their own scheduling or rate limiting.
    ///
    /// # Errors
    ///
    /// Returns [`ReconnectPolicyError`] when `initial_delay` exceeds
    /// `max_delay`.
    pub fn new(
        max_attempts: u32,
        initial_delay: Duration,
        max_delay: Duration,
    ) -> Result<Self, ReconnectPolicyError> {
        if initial_delay > max_delay {
            Err(ReconnectPolicyError::InitialDelayExceedsMaximum)
        } else {
            Ok(Self {
                max_attempts,
                initial_delay,
                max_delay,
            })
        }
    }

    /// Maximum number of retries after one run of transport losses.
    #[must_use]
    pub const fn max_attempts(self) -> u32 {
        self.max_attempts
    }

    /// Delay before the first retry.
    #[must_use]
    pub const fn initial_delay(self) -> Duration {
        self.initial_delay
    }

    /// Upper bound applied to every exponential-backoff delay.
    #[must_use]
    pub const fn max_delay(self) -> Duration {
        self.max_delay
    }

    fn delay_for_attempt(self, attempt: u32) -> Option<Duration> {
        if attempt == 0 || attempt > self.max_attempts {
            return None;
        }

        let shift = attempt - 1;
        let multiplier = 1_u32.checked_shl(shift).unwrap_or(u32::MAX);
        Some(
            self.initial_delay
                .saturating_mul(multiplier)
                .min(self.max_delay),
        )
    }
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            initial_delay: Duration::from_millis(250),
            max_delay: Duration::from_secs(8),
        }
    }
}

/// Invalid reconnect-policy configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconnectPolicyError {
    /// The first delay is already larger than the configured delay cap.
    InitialDelayExceedsMaximum,
}

impl fmt::Display for ReconnectPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("initial reconnect delay exceeds maximum delay")
    }
}

impl std::error::Error for ReconnectPolicyError {}

/// Action selected after a recoverable transport loss.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconnectDecision {
    /// Retry after a bounded exponential-backoff delay.
    RetryAfter {
        /// One-based consecutive retry number.
        attempt: u32,
        /// Delay the caller should apply before starting the retry.
        delay: Duration,
        /// Last accepted sequence, suitable for a transport resume request.
        resume_after: Option<u64>,
    },
    /// No retry remains; the session has transitioned to closed.
    GiveUp,
}

/// Invalid operation for the current session lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionError {
    /// The operation is not legal in the supplied phase.
    InvalidTransition {
        /// Phase in which the rejected operation was requested.
        phase: SessionPhase,
        /// Stable name of the rejected operation.
        operation: &'static str,
    },
    /// A first connection cannot resume a stream that does not yet exist.
    CannotResumeInitialConnection,
    /// A resumed stream must retain the exact prior negotiated agreement.
    AgreementChangedDuringResume,
    /// The successful-connection counter can no longer be incremented.
    ConnectionGenerationExhausted,
}

impl fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTransition { phase, operation } => {
                write!(formatter, "cannot {operation} while session is {phase:?}")
            }
            Self::CannotResumeInitialConnection => {
                formatter.write_str("initial connection cannot resume a prior stream")
            }
            Self::AgreementChangedDuringResume => {
                formatter.write_str("resumed stream changed its negotiated agreement")
            }
            Self::ConnectionGenerationExhausted => {
                formatter.write_str("connection generation counter is exhausted")
            }
        }
    }
}

impl std::error::Error for SessionError {}

/// Deterministic state machine for one logical host/display session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    phase: SessionPhase,
    agreement: Option<NegotiatedSession>,
    connection_generation: u64,
    reconnect_attempts: u32,
    highest_sequence: Option<u64>,
}

impl Session {
    /// Construct an idle session with no negotiated agreement or sequence state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            phase: SessionPhase::Idle,
            agreement: None,
            connection_generation: 0,
            reconnect_attempts: 0,
            highest_sequence: None,
        }
    }

    /// Current lifecycle phase.
    #[must_use]
    pub const fn phase(&self) -> SessionPhase {
        self.phase
    }

    /// Most recently established agreement, retained across transient loss.
    #[must_use]
    pub const fn agreement(&self) -> Option<NegotiatedSession> {
        self.agreement
    }

    /// Number of transports successfully established for this logical session.
    #[must_use]
    pub const fn connection_generation(&self) -> u64 {
        self.connection_generation
    }

    /// Consecutive retries scheduled since the most recent establishment.
    #[must_use]
    pub const fn reconnect_attempts(&self) -> u32 {
        self.reconnect_attempts
    }

    /// Highest accepted sequence in the current logical stream.
    #[must_use]
    pub const fn highest_sequence(&self) -> Option<u64> {
        self.highest_sequence
    }

    /// Begin the first hello and capability exchange.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::InvalidTransition`] unless the session is idle.
    pub fn start(&mut self) -> Result<(), SessionError> {
        self.require_phase(SessionPhase::Idle, "start")?;
        self.phase = SessionPhase::Negotiating;
        Ok(())
    }

    /// Mark negotiation complete and make the transport active.
    ///
    /// Restarting clears the receive cursor. Resuming retains the cursor and
    /// requires an agreement identical to the previous active transport.
    /// Successful establishment resets the consecutive retry count.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError`] when negotiation is not in progress, the first
    /// connection requests resume semantics, a resumed agreement changed, or
    /// the connection-generation counter is exhausted.
    pub fn establish(
        &mut self,
        agreement: NegotiatedSession,
        continuity: StreamContinuity,
    ) -> Result<(), SessionError> {
        self.require_phase(SessionPhase::Negotiating, "establish")?;
        let next_generation = self
            .connection_generation
            .checked_add(1)
            .ok_or(SessionError::ConnectionGenerationExhausted)?;

        match continuity {
            StreamContinuity::Restart => self.highest_sequence = None,
            StreamContinuity::Resume if self.connection_generation == 0 => {
                return Err(SessionError::CannotResumeInitialConnection);
            }
            StreamContinuity::Resume if self.agreement != Some(agreement) => {
                return Err(SessionError::AgreementChangedDuringResume);
            }
            StreamContinuity::Resume => {}
        }

        self.connection_generation = next_generation;
        self.agreement = Some(agreement);
        self.reconnect_attempts = 0;
        self.phase = SessionPhase::Active;
        Ok(())
    }

    /// Classify and, when advancing, record an inbound stream sequence.
    ///
    /// Duplicate and stale values never move the receive cursor backwards.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::InvalidTransition`] unless the session is active.
    pub fn observe_sequence(&mut self, sequence: u64) -> Result<SequenceDisposition, SessionError> {
        self.require_phase(SessionPhase::Active, "observe sequence")?;

        let disposition = match self.highest_sequence {
            None => {
                self.highest_sequence = Some(sequence);
                SequenceDisposition::Accepted { skipped: 0 }
            }
            Some(highest) if sequence > highest => {
                self.highest_sequence = Some(sequence);
                SequenceDisposition::Accepted {
                    skipped: sequence - highest - 1,
                }
            }
            Some(highest) if sequence == highest => SequenceDisposition::Duplicate,
            Some(highest) => SequenceDisposition::Stale {
                highest_accepted: highest,
            },
        };

        Ok(disposition)
    }

    /// Record a recoverable transport loss.
    ///
    /// The agreement and receive cursor are retained so a later establishment
    /// may explicitly resume them.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::InvalidTransition`] unless a transport is active
    /// or negotiation is in progress.
    pub fn transport_lost(&mut self) -> Result<(), SessionError> {
        if !matches!(self.phase, SessionPhase::Active | SessionPhase::Negotiating) {
            return Err(self.invalid_transition("record transport loss"));
        }
        self.phase = SessionPhase::WaitingToReconnect;
        Ok(())
    }

    /// Schedule the next reconnect or close after exhausting the policy.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::InvalidTransition`] unless the session is
    /// waiting to reconnect.
    pub fn schedule_reconnect(
        &mut self,
        policy: ReconnectPolicy,
    ) -> Result<ReconnectDecision, SessionError> {
        self.require_phase(SessionPhase::WaitingToReconnect, "schedule reconnect")?;

        let Some(attempt) = self.reconnect_attempts.checked_add(1) else {
            self.phase = SessionPhase::Closed;
            return Ok(ReconnectDecision::GiveUp);
        };
        let Some(delay) = policy.delay_for_attempt(attempt) else {
            self.phase = SessionPhase::Closed;
            return Ok(ReconnectDecision::GiveUp);
        };

        self.reconnect_attempts = attempt;
        self.phase = SessionPhase::ReconnectScheduled;
        Ok(ReconnectDecision::RetryAfter {
            attempt,
            delay,
            resume_after: self.highest_sequence,
        })
    }

    /// Begin negotiation after the caller has honored a scheduled retry delay.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::InvalidTransition`] unless a reconnect is
    /// scheduled.
    pub fn begin_reconnect(&mut self) -> Result<(), SessionError> {
        self.require_phase(SessionPhase::ReconnectScheduled, "begin reconnect")?;
        self.phase = SessionPhase::Negotiating;
        Ok(())
    }

    /// Permanently close the logical session.
    ///
    /// This operation is idempotent. Diagnostic agreement and sequence state
    /// remain available through their accessors.
    pub fn close(&mut self) {
        self.phase = SessionPhase::Closed;
    }

    fn require_phase(
        &self,
        expected: SessionPhase,
        operation: &'static str,
    ) -> Result<(), SessionError> {
        if self.phase == expected {
            Ok(())
        } else {
            Err(self.invalid_transition(operation))
        }
    }

    const fn invalid_transition(&self, operation: &'static str) -> SessionError {
        SessionError::InvalidTransition {
            phase: self.phase,
            operation,
        }
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ladoflow_protocol::{Capabilities, CodecSet, FeatureFlags, Hello, InputCapabilities, Role};

    use crate::{NegotiatedSession, negotiate};

    use super::{
        ReconnectDecision, ReconnectPolicy, SequenceDisposition, Session, SessionError,
        SessionPhase, StreamContinuity,
    };

    fn agreement(width: u16) -> NegotiatedSession {
        let host = Hello::new(1, 1, Role::Host, [1; 16], "test host").expect("valid hello");
        let display =
            Hello::new(1, 1, Role::Display, [2; 16], "test display").expect("valid hello");
        let capabilities = Capabilities::new(
            width,
            1080,
            60_000,
            20_000,
            CodecSet::H264,
            InputCapabilities::default(),
            FeatureFlags::default(),
        )
        .expect("valid capabilities");
        negotiate(&host, capabilities, &display, capabilities).expect("compatible endpoints")
    }

    #[test]
    fn active_session_classifies_sequence_progress_without_regression() {
        let mut session = Session::new();
        session.start().expect("start session");
        session
            .establish(agreement(1920), StreamContinuity::Restart)
            .expect("establish session");

        assert_eq!(
            session.observe_sequence(10),
            Ok(SequenceDisposition::Accepted { skipped: 0 })
        );
        assert_eq!(
            session.observe_sequence(13),
            Ok(SequenceDisposition::Accepted { skipped: 2 })
        );
        assert_eq!(
            session.observe_sequence(13),
            Ok(SequenceDisposition::Duplicate)
        );
        assert_eq!(
            session.observe_sequence(12),
            Ok(SequenceDisposition::Stale {
                highest_accepted: 13
            })
        );
        assert_eq!(session.highest_sequence(), Some(13));
    }

    #[test]
    fn reconnect_uses_capped_backoff_and_preserves_resume_cursor() {
        let policy =
            ReconnectPolicy::new(4, Duration::from_millis(100), Duration::from_millis(250))
                .expect("valid policy");
        let mut session = Session::new();
        session.start().expect("start session");
        session
            .establish(agreement(1920), StreamContinuity::Restart)
            .expect("establish session");
        session.observe_sequence(41).expect("record sequence");

        for (attempt, expected_delay) in [100_u64, 200, 250, 250].into_iter().enumerate() {
            session.transport_lost().expect("record loss");
            assert_eq!(
                session.schedule_reconnect(policy),
                Ok(ReconnectDecision::RetryAfter {
                    attempt: u32::try_from(attempt + 1).expect("small attempt"),
                    delay: Duration::from_millis(expected_delay),
                    resume_after: Some(41),
                })
            );
            session.begin_reconnect().expect("begin retry");
        }

        session.transport_lost().expect("record final loss");
        assert_eq!(
            session.schedule_reconnect(policy),
            Ok(ReconnectDecision::GiveUp)
        );
        assert_eq!(session.phase(), SessionPhase::Closed);
    }

    #[test]
    fn resume_preserves_sequence_but_restart_clears_it() {
        let current = agreement(1920);
        let mut session = Session::new();
        session.start().expect("start session");
        session
            .establish(current, StreamContinuity::Restart)
            .expect("establish session");
        session.observe_sequence(7).expect("record sequence");
        session.transport_lost().expect("record loss");
        session
            .schedule_reconnect(ReconnectPolicy::default())
            .expect("schedule retry");
        session.begin_reconnect().expect("begin retry");
        session
            .establish(current, StreamContinuity::Resume)
            .expect("resume session");
        assert_eq!(session.highest_sequence(), Some(7));
        assert_eq!(session.connection_generation(), 2);

        session.transport_lost().expect("record loss");
        session
            .schedule_reconnect(ReconnectPolicy::default())
            .expect("schedule retry");
        session.begin_reconnect().expect("begin retry");
        session
            .establish(agreement(1280), StreamContinuity::Restart)
            .expect("restart session");
        assert_eq!(session.highest_sequence(), None);
    }

    #[test]
    fn rejects_invalid_transitions_and_changed_resume_agreement() {
        let current = agreement(1920);
        let mut session = Session::new();
        assert!(matches!(
            session.observe_sequence(1),
            Err(SessionError::InvalidTransition {
                phase: SessionPhase::Idle,
                ..
            })
        ));

        session.start().expect("start session");
        assert_eq!(
            session.establish(current, StreamContinuity::Resume),
            Err(SessionError::CannotResumeInitialConnection)
        );
        session
            .establish(current, StreamContinuity::Restart)
            .expect("establish session");
        session.transport_lost().expect("record loss");
        session
            .schedule_reconnect(ReconnectPolicy::default())
            .expect("schedule retry");
        session.begin_reconnect().expect("begin retry");
        assert_eq!(
            session.establish(agreement(1280), StreamContinuity::Resume),
            Err(SessionError::AgreementChangedDuringResume)
        );
        assert_eq!(session.phase(), SessionPhase::Negotiating);
    }

    #[test]
    fn exhausted_attempt_counter_cannot_schedule_forever() {
        let mut session = Session::new();
        session.phase = SessionPhase::WaitingToReconnect;
        session.reconnect_attempts = u32::MAX;
        let policy =
            ReconnectPolicy::new(u32::MAX, Duration::ZERO, Duration::ZERO).expect("valid policy");

        assert_eq!(
            session.schedule_reconnect(policy),
            Ok(ReconnectDecision::GiveUp)
        );
        assert_eq!(session.phase(), SessionPhase::Closed);
    }
}
