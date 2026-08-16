//! Permission bridge: the registry that parks `session/request_permission`
//! decisions between the connection task (which holds the responder) and
//! whoever answers for the user (phase D surfaces them via
//! `AgentSink::permission_request`; `respond_agent_permission` resolves
//! them). Also owns the normative cancellation step: every pending request
//! of a session resolves with `Cancelled` before the `CancelNotification`
//! goes out (schema doc on `RequestPermissionOutcome::Cancelled`).
//!
//! The UI payload types (`AgentPermissionPayload` / `AgentPermissionOption`)
//! live in `events.rs` (phase D2) — this module only owns the registry and
//! the resolve logic.

use agent_client_protocol::schema::v1::{
    PermissionOptionId, RequestPermissionOutcome, SelectedPermissionOutcome,
};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::Mutex;

/// A parked permission decision: the oneshot the connection task's responder
/// task awaits, plus the precomputed allow-option id.
pub struct PermissionPending {
    pub tx: tokio::sync::oneshot::Sender<RequestPermissionOutcome>,
    /// First option whose kind is `allow_once` / `allow_always`, resolved at
    /// request time so `respond(allow=true)` can select it without holding
    /// the request.
    pub allow_option_id: Option<PermissionOptionId>,
}

/// FIFO queues of pending permission requests, keyed by session id.
pub struct PermissionRegistry {
    pub map: Arc<Mutex<HashMap<String, VecDeque<PermissionPending>>>>,
}

impl PermissionRegistry {
    pub fn new() -> Self {
        Self {
            map: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Parks a new pending request at the back of the session's queue.
    pub async fn push(&self, session_id: &str, pending: PermissionPending) {
        self.map
            .lock()
            .await
            .entry(session_id.to_string())
            .or_default()
            .push_back(pending);
    }

    /// Resolves the FRONT of the session's queue with the user's decision.
    /// `allow=true` selects the first allow-kind option the agent offered
    /// (error if it offered none — the agent must present an option to
    /// select); `allow=false` resolves with `Cancelled`.
    pub async fn respond(&self, session_id: &str, allow: bool) -> Result<(), String> {
        let mut map = self.map.lock().await;
        let queue = map
            .get_mut(session_id)
            .ok_or_else(|| format!("no pending permission request for session {session_id}"))?;
        // Validate BEFORE popping: an error must leave the pending parked so
        // the user can still deny it.
        let front = queue
            .front()
            .ok_or_else(|| format!("no pending permission request for session {session_id}"))?;
        if allow && front.allow_option_id.is_none() {
            return Err("the agent offered no allow option for this permission request".into());
        }
        let pending = queue.pop_front().expect("front exists — checked above");
        let outcome = if allow {
            RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
                pending.allow_option_id.expect("checked above"),
            ))
        } else {
            RequestPermissionOutcome::Cancelled
        };
        let _ = pending.tx.send(outcome);
        Ok(())
    }

    /// Cancellation step 1 (normative MUST): resolve EVERY pending request
    /// of the session with `Cancelled`, before the driver sends the
    /// `CancelNotification`. A second cancel finds an empty queue — no-op.
    pub async fn drain_cancelled(&self, session_id: &str) {
        let mut map = self.map.lock().await;
        if let Some(queue) = map.get_mut(session_id) {
            for pending in queue.drain(..) {
                let _ = pending.tx.send(RequestPermissionOutcome::Cancelled);
            }
        }
    }
}

impl Default for PermissionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::v1::RequestPermissionOutcome;
    use tokio::sync::oneshot;

    fn pending(
        allow: Option<&str>,
    ) -> (
        PermissionPending,
        oneshot::Receiver<RequestPermissionOutcome>,
    ) {
        let (tx, rx) = oneshot::channel();
        (
            PermissionPending {
                tx,
                allow_option_id: allow.map(PermissionOptionId::new),
            },
            rx,
        )
    }

    #[tokio::test]
    async fn respond_resolves_front_of_queue() {
        let reg = PermissionRegistry::new();
        let (p1, mut rx1) = pending(Some("allow_once"));
        let (p2, mut rx2) = pending(None);
        reg.push("s1", p1).await;
        reg.push("s1", p2).await;

        // allow=true resolves the FIRST pending with Selected(allow option)
        reg.respond("s1", true).await.expect("first resolves");
        let outcome = rx1.await.expect("first oneshot fires");
        match outcome {
            RequestPermissionOutcome::Selected(sel) => {
                assert_eq!(sel.option_id.to_string(), "allow_once");
            }
            other => panic!("expected Selected, got {other:?}"),
        }

        // allow=false resolves the SECOND with Cancelled
        reg.respond("s1", false).await.expect("second resolves");
        let outcome = rx2.await.expect("second oneshot fires");
        assert!(
            matches!(outcome, RequestPermissionOutcome::Cancelled),
            "deny resolves Cancelled: {outcome:?}"
        );
    }

    #[tokio::test]
    async fn empty_queue_is_an_error() {
        let reg = PermissionRegistry::new();
        let err = reg
            .respond("s1", true)
            .await
            .expect_err("empty queue errors");
        assert!(err.contains("no pending permission request"), "{err}");
    }

    #[tokio::test]
    async fn allow_without_allow_option_is_an_error_and_keeps_the_queue() {
        let reg = PermissionRegistry::new();
        let (p, mut rx) = pending(None); // agent offered no allow-kind option
        reg.push("s1", p).await;
        let err = reg
            .respond("s1", true)
            .await
            .expect_err("no option to select");
        assert!(err.contains("no allow option"), "{err}");
        // the pending is still parked and can still be denied
        reg.respond("s1", false).await.expect("deny still works");
        assert!(matches!(
            rx.await.expect("oneshot fires"),
            RequestPermissionOutcome::Cancelled
        ));
    }

    #[tokio::test]
    async fn drain_cancelled_resolves_every_pending() {
        let reg = PermissionRegistry::new();
        let (p1, mut rx1) = pending(Some("allow_once"));
        let (p2, mut rx2) = pending(None);
        let (p3, mut rx3) = pending(Some("allow_always"));
        reg.push("s1", p1).await;
        reg.push("s1", p2).await;
        reg.push("s2", p3).await; // other session untouched

        reg.drain_cancelled("s1").await;
        for rx in [&mut rx1, &mut rx2] {
            let outcome = rx.await.expect("oneshot fires");
            assert!(
                matches!(outcome, RequestPermissionOutcome::Cancelled),
                "drain resolves Cancelled: {outcome:?}"
            );
        }
        // s2's pending survives (the cancel targets one session)
        assert!(
            rx3.try_recv().is_err(),
            "other session's pending is not drained"
        );
        // second drain is a no-op
        reg.drain_cancelled("s1").await;
    }
}
