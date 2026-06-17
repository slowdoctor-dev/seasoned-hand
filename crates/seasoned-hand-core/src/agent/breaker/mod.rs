use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::verifier::BreakerKind;

#[derive(Debug, Clone)]
pub struct CircuitBreaker {
    state: Arc<Mutex<BreakerState>>,
}

#[derive(Debug, Default, Clone)]
struct BreakerState {
    stuck_count: u32,
    cost_cents: u32,
    iteration_count: u32,
    recent_obs_ok: VecDeque<bool>,
    armed: bool,
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new()
    }
}

impl CircuitBreaker {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(BreakerState {
                armed: true,
                ..BreakerState::default()
            })),
        }
    }

    pub async fn note_stuck_and_check(&self, count: u32) -> Option<BreakerKind> {
        let mut s = self.state.lock().await;
        s.stuck_count = count;
        if s.armed && s.stuck_count >= 4 {
            s.armed = false;
            return Some(BreakerKind::Stuck);
        }
        None
    }

    pub async fn note_cost_and_check(
        &self,
        cost_cents: u32,
        cap_cents: u32,
    ) -> Option<BreakerKind> {
        let mut s = self.state.lock().await;
        s.cost_cents = cost_cents;
        if s.armed && s.cost_cents >= cap_cents {
            s.armed = false;
            return Some(BreakerKind::Cost);
        }
        None
    }

    pub async fn note_iteration_and_check(
        &self,
        iteration_count: u32,
        max_steps: u32,
    ) -> Option<BreakerKind> {
        let mut s = self.state.lock().await;
        s.iteration_count = iteration_count;
        if s.armed && s.iteration_count >= max_steps {
            s.armed = false;
            return Some(BreakerKind::MaxSteps);
        }
        None
    }

    pub async fn note_observation_and_check(&self, ok: bool) -> Option<BreakerKind> {
        let mut s = self.state.lock().await;
        s.recent_obs_ok.push_back(ok);
        if s.recent_obs_ok.len() > 10 {
            let _ = s.recent_obs_ok.pop_front();
        }
        let failures = s.recent_obs_ok.iter().filter(|v| !**v).count();
        if s.armed && failures >= 5 {
            s.armed = false;
            return Some(BreakerKind::ErrorRate);
        }
        None
    }

    // Issue #23: breakers are **one-shot per session** — the agent loop trips a
    // breaker and finalizes the session; it never re-arms a tripped breaker
    // mid-run. These reset helpers are intentionally not wired into the loop;
    // they exist for explicit operator/test-driven recovery and for a future
    // resumable-breaker policy. Kept (rather than removed) as the deliberate
    // state-transition surface for that policy.
    pub async fn rearm(&self) {
        let mut s = self.state.lock().await;
        s.armed = true;
    }

    pub async fn reset_stuck(&self) {
        let mut s = self.state.lock().await;
        s.stuck_count = 0;
    }

    pub async fn reset_error_rate(&self) {
        let mut s = self.state.lock().await;
        s.recent_obs_ok.clear();
    }
}

#[derive(Default, Clone)]
pub struct BreakerRegistry {
    inner: Arc<Mutex<HashMap<String, CircuitBreaker>>>,
}

impl BreakerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn for_session(&self, session_id: &str) -> CircuitBreaker {
        let mut guard = self.inner.lock().await;
        guard
            .entry(session_id.to_string())
            .or_insert_with(CircuitBreaker::new)
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn breaker_trips_on_stuck_at_4() {
        let b = CircuitBreaker::new();
        assert_eq!(b.note_stuck_and_check(3).await, None);
        assert_eq!(b.note_stuck_and_check(4).await, Some(BreakerKind::Stuck));
    }

    #[tokio::test]
    async fn breaker_trips_on_cost_at_cap() {
        let b = CircuitBreaker::new();
        assert_eq!(b.note_cost_and_check(4, 5).await, None);
        assert_eq!(b.note_cost_and_check(5, 5).await, Some(BreakerKind::Cost));
    }

    #[tokio::test]
    async fn breaker_trips_on_max_steps() {
        let b = CircuitBreaker::new();
        assert_eq!(b.note_iteration_and_check(2, 3).await, None);
        assert_eq!(
            b.note_iteration_and_check(3, 3).await,
            Some(BreakerKind::MaxSteps)
        );
    }

    #[tokio::test]
    async fn breaker_trips_on_error_rate_5_of_10() {
        let b = CircuitBreaker::new();
        for i in 0..9 {
            let ok = i % 2 == 0;
            let _ = b.note_observation_and_check(ok).await;
        }
        assert_eq!(
            b.note_observation_and_check(false).await,
            Some(BreakerKind::ErrorRate)
        );
    }
}
