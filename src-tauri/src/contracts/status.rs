use crate::contracts::errors::{RecordingErrorCode, RecoveryHint};
use crate::contracts::events::{PipelineMode, PipelinePhase};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

pub const SESSION_STATUS_QUEUE_CAPACITY: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStatus {
    pub state: PipelinePhase,
    pub mode: PipelineMode,
    pub error_code: Option<RecordingErrorCode>,
    pub error_hint: RecoveryHint,
    pub session_id: u64,
    pub seq: u64,
    pub phase_elapsed_ms: u64,
    pub source: String,
}

impl SessionStatus {
    pub const fn with_defaults(
        state: PipelinePhase,
        mode: PipelineMode,
        session_id: u64,
        seq: u64,
        source: String,
    ) -> Self {
        Self {
            state,
            mode,
            error_code: None,
            error_hint: RecoveryHint::NoRecovery,
            session_id,
            seq,
            phase_elapsed_ms: 0,
            source,
        }
    }
}

impl fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "session_status(session={},seq={},state={},mode={},error_code={:?},error_hint={},phase_elapsed_ms={},source={})",
            self.session_id,
            self.seq,
            self.state,
            self.mode,
            self.error_code,
            self.error_hint,
            self.phase_elapsed_ms,
            self.source
        )
    }
}

#[derive(Debug)]
struct StatusChannelState {
    queue: VecDeque<SessionStatus>,
    closed: bool,
    dropped_oldest: u64,
}

#[derive(Clone, Debug)]
pub struct SessionStatusSender {
    inner: Arc<StatusChannelInner>,
}

#[derive(Debug)]
pub struct SessionStatusReceiver {
    inner: Arc<StatusChannelInner>,
}

#[derive(Debug)]
struct StatusChannelInner {
    capacity: usize,
    state: Mutex<StatusChannelState>,
    notify: Notify,
}

pub fn bounded_status_channel(capacity: usize) -> (SessionStatusSender, SessionStatusReceiver) {
    let capacity = capacity.max(1);
    let inner = Arc::new(StatusChannelInner {
        capacity,
        state: Mutex::new(StatusChannelState {
            queue: VecDeque::with_capacity(capacity),
            closed: false,
            dropped_oldest: 0,
        }),
        notify: Notify::new(),
    });

    (
        SessionStatusSender {
            inner: Arc::clone(&inner),
        },
        SessionStatusReceiver { inner },
    )
}

impl SessionStatusSender {
    pub fn send(&self, status: SessionStatus) -> Result<StatusPublishOutcome, StatusChannelClosed> {
        let mut state = self.inner.state.lock().map_err(|_| StatusChannelClosed)?;

        if state.closed {
            return Err(StatusChannelClosed);
        }

        // Overflow policy: drop the oldest queued status and enqueue the newest.
        // This bounds memory while preserving monotonic source order for statuses
        // that observers actually receive, and keeps UI/tray consumers converging
        // on the latest coherent state even when they are slow.
        let dropped_oldest = if state.queue.len() >= self.inner.capacity {
            state.queue.pop_front();
            state.dropped_oldest = state.dropped_oldest.saturating_add(1);
            true
        } else {
            false
        };

        state.queue.push_back(status);
        let dropped_total = state.dropped_oldest;
        drop(state);
        self.inner.notify.notify_one();

        Ok(StatusPublishOutcome {
            dropped_oldest,
            dropped_total,
        })
    }
}

impl Drop for SessionStatusSender {
    fn drop(&mut self) {
        if Arc::strong_count(&self.inner) == 2 {
            if let Ok(mut state) = self.inner.state.lock() {
                state.closed = true;
            }
            self.inner.notify.notify_waiters();
        }
    }
}

impl SessionStatusReceiver {
    pub async fn recv(&mut self) -> Option<SessionStatus> {
        loop {
            let notified = self.inner.notify.notified();

            {
                let mut state = self.inner.state.lock().ok()?;
                if let Some(status) = state.queue.pop_front() {
                    return Some(status);
                }
                if state.closed {
                    return None;
                }
            }

            notified.await;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusPublishOutcome {
    pub dropped_oldest: bool,
    pub dropped_total: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusChannelClosed;
