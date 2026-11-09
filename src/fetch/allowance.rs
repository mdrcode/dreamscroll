//! What a run may spend, and why it may not.
//!
//! A run that may never fetch has *no allowance* rather than one that refuses — the absence of a door
//! rather than a closed one. That is what keeps [`Refusal`] honest: both of its reasons are things
//! that can happen mid-run, so anything that cannot happen mid-run is not a refusal.
//!
//! Rate limiting and the request budget are not the door's business — they are the caller's policy,
//! enforced by the door because the door is the only place a call can happen. So the state lives
//! here, one `Allowance` is built per run and passed into [`crate::fetch::Gateway`], and the gateway
//! asks it before every transport call and never looks inside it.
//!
//! One type rather than two pieces of state because both refusals are the same question —
//! *may I make another call?* — with different reasons. Keeping them apart is how a 429 ended up
//! with no check inside the door and only one between providers.

use std::mem;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

/// Why the door closed. Both reasons arise mid-run; a run that may never fetch has no door at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The run spent its calls.
    Budget,
    /// The provider answered `429`. `retry_after_secs` is what it asked us to wait, when it said —
    /// `None` is "no `Retry-After`", not "wait zero seconds".
    RateLimited { retry_after_secs: Option<u64> },
}

impl Refusal {
    /// How long the provider asked us to wait, when it said and when that is why it refused.
    pub fn retry_after_secs(self) -> Option<u64> {
        match self {
            Self::RateLimited { retry_after_secs } => retry_after_secs,
            Self::Budget => None,
        }
    }
}

/// One run's permission to call: paced, budgeted, and revocable by the provider.
pub struct Allowance {
    pacer: Mutex<Pacer>,
    remaining_calls: Mutex<usize>,
    /// Set once the budget is gone, so [`Self::refused`] can answer without a lock.
    is_spent: AtomicBool,
    /// The provider's `429`, latched for the rest of the run. The inner `None` is "it gave no
    /// `Retry-After`"; the outer one is "no `429` has happened".
    rate_limit: OnceLock<Option<u64>>,
}

impl Allowance {
    /// `max_calls` transport calls for the run, with at least `min_interval` between them.
    pub fn new(max_calls: usize, min_interval: Duration) -> Self {
        Self {
            pacer: Mutex::new(Pacer::new(min_interval)),
            remaining_calls: Mutex::new(max_calls),
            is_spent: AtomicBool::new(false),
            rate_limit: OnceLock::new(),
        }
    }

    /// One transport call, paid for: waits out the floor, then decrements the budget.
    ///
    /// Every refusal is sticky, so a `429` on one batch stops every later stage rather than being
    /// rediscovered call by call — which is what the provider's ban rule punishes.
    pub async fn take(&self) -> Result<(), Refusal> {
        if let Some(refusal) = self.refused() {
            return Err(refusal);
        }
        {
            let mut remaining_calls = self.remaining_calls.lock().await;
            if *remaining_calls == 0 {
                self.is_spent.store(true, Ordering::Relaxed);
                return Err(Refusal::Budget);
            }
            *remaining_calls -= 1;
        }
        self.pacer.lock().await.pace().await;
        Ok(())
    }

    /// Whether the run is over, and why. Asked before every call, and once more at the end to say
    /// what a run ran into.
    pub fn refused(&self) -> Option<Refusal> {
        if let Some(retry_after_secs) = self.rate_limit.get() {
            return Some(Refusal::RateLimited {
                retry_after_secs: *retry_after_secs,
            });
        }
        self.is_spent
            .load(Ordering::Relaxed)
            .then_some(Refusal::Budget)
    }

    /// The provider asked us to stop. Sticky for the rest of the run, and never cached as a fact.
    pub fn rate_limited(&self, retry_after_secs: Option<u64>) {
        let _ = self.rate_limit.set(retry_after_secs);
    }
}

/// A floor between transport calls.
///
/// A query endpoint may meter processing time rather than request count, but a floor is the simplest
/// thing that cannot be wrong. It lives for a whole run, so two passes back to back do not each
/// start afresh.
struct Pacer {
    after: Option<Instant>,
    interval: Duration,
}

impl Pacer {
    fn new(interval: Duration) -> Self {
        Self {
            after: None,
            interval,
        }
    }

    async fn pace(&mut self) {
        if let Some(previous) = mem::take(&mut self.after) {
            let ready = previous + self.interval;
            let now = Instant::now();
            if ready > now {
                tokio::time::sleep(ready - now).await;
            }
        }
        self.after = Some(Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The budget is spent before the call, and the refusal outlives it: a run that ran out cannot
    /// quietly start calling again on the next group.
    #[tokio::test]
    async fn a_budget_is_spent_once_per_call_and_the_refusal_sticks() {
        let allowance = Allowance::new(2, Duration::ZERO);

        assert_eq!(allowance.refused(), None);
        assert_eq!(allowance.take().await, Ok(()));
        assert_eq!(allowance.take().await, Ok(()));
        assert_eq!(allowance.take().await, Err(Refusal::Budget));
        assert_eq!(allowance.refused(), Some(Refusal::Budget));
    }

    /// A 429 is an instruction for the rest of the run, not a fact about one call.
    #[tokio::test]
    async fn a_rate_limit_refuses_every_later_call() {
        let allowance = Allowance::new(10, Duration::ZERO);
        allowance.rate_limited(Some(30));

        assert_eq!(
            allowance.take().await,
            Err(Refusal::RateLimited {
                retry_after_secs: Some(30)
            })
        );
        assert_eq!(
            allowance.refused(),
            Some(Refusal::RateLimited {
                retry_after_secs: Some(30)
            })
        );
        assert_eq!(
            Refusal::RateLimited {
                retry_after_secs: Some(30)
            }
            .retry_after_secs(),
            Some(30)
        );
        assert_eq!(Refusal::Budget.retry_after_secs(), None);
    }

    /// A `429` without a `Retry-After` still stops the run: the provider refused, it just did not say
    /// for how long. Only the time is unknown, not the refusal.
    #[tokio::test]
    async fn a_rate_limit_without_a_retry_after_still_latches() {
        let allowance = Allowance::new(10, Duration::ZERO);
        allowance.rate_limited(None);

        assert_eq!(
            allowance.refused(),
            Some(Refusal::RateLimited {
                retry_after_secs: None
            })
        );
        assert_eq!(allowance.take().await, Err(Refusal::RateLimited { retry_after_secs: None }));
        assert_eq!(
            Refusal::RateLimited {
                retry_after_secs: None
            }
            .retry_after_secs(),
            None
        );
    }

    /// The floor is between calls: a single lookup must not pay it, and the second call must.
    /// Timing-based, so the numbers are loose enough to survive a busy machine.
    #[tokio::test]
    async fn the_pacer_waits_between_calls_and_not_before_the_first() {
        let allowance = Allowance::new(3, Duration::from_millis(60));

        let start = Instant::now();
        allowance.take().await.expect("the first call");
        assert!(
            start.elapsed() < Duration::from_millis(60),
            "the first call waits for nothing: {:?}",
            start.elapsed()
        );

        allowance.take().await.expect("the second call");
        assert!(
            start.elapsed() >= Duration::from_millis(60),
            "the second call waits the floor: {:?}",
            start.elapsed()
        );
    }
}
