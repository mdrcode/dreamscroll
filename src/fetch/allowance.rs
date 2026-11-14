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
//! One type because both answer *may I make another call?*: the budget is run-wide, while a `429`
//! closes only the provider that sent it.

use std::collections::HashMap;
use std::mem;
use std::sync::Mutex as StdMutex;
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

/// What one run's [`Allowance`] is built from: calls allowed, and the pace between them.
///
/// Deployment policy, held by the wiring; an `Allowance` cannot be reused across runs because it
/// carries the spent counter and the latched refusals.
#[derive(Debug, Clone, Copy)]
pub struct AllowanceConfig {
    pub max_calls: usize,
    pub min_interval: Duration,
}

/// One run's permission to call: paced, budgeted, and revocable per provider.
pub struct Allowance {
    pacer: Mutex<Pacer>,
    remaining_calls: Mutex<usize>,
    /// Set once the budget is gone, so [`Self::refused`] can answer without a lock.
    is_spent: AtomicBool,
    /// Each provider's `429`, latched for the rest of the run. The inner `None` is "it gave no
    /// `Retry-After`"; the map entry is the fact that it refused.
    rate_limits: StdMutex<HashMap<String, Option<u64>>>,
}

impl Allowance {
    /// The allowance a configured policy grants: one run, built fresh.
    pub fn configured(config: AllowanceConfig) -> Self {
        Self::new(config.max_calls, config.min_interval)
    }

    /// `max_calls` transport calls for the run, with at least `min_interval` between them.
    pub fn new(max_calls: usize, min_interval: Duration) -> Self {
        Self {
            pacer: Mutex::new(Pacer::new(min_interval)),
            remaining_calls: Mutex::new(max_calls),
            is_spent: AtomicBool::new(false),
            rate_limits: StdMutex::new(HashMap::new()),
        }
    }

    /// One batch for `provider`, paid for: waits out the floor, then decrements the budget by what
    /// the batch costs.
    ///
    /// All or nothing. A batch that fans out into two HTTP requests costs two, and a batch that
    /// cannot be afforded is refused *before* anything is spent — otherwise a run one unit short
    /// would have paid for a call it never made and reported a budget it had not finished.
    ///
    /// A provider's `429` is sticky for that provider, so later stages do not ask it again. Other
    /// providers are independent services and keep their own permission.
    pub async fn take_for(&self, provider: &str, calls: usize) -> Result<(), Refusal> {
        if let Some(refusal) = self.refused_for(provider) {
            return Err(refusal);
        }
        {
            let mut remaining_calls = self.remaining_calls.lock().await;
            if *remaining_calls < calls {
                self.is_spent.store(true, Ordering::Relaxed);
                return Err(Refusal::Budget);
            }
            *remaining_calls -= calls;
        }
        self.pacer.lock().await.pace().await;
        Ok(())
    }

    /// Whether this provider is refused, and why. The run-wide budget applies to every provider.
    pub fn refused_for(&self, provider: &str) -> Option<Refusal> {
        if self.is_spent.load(Ordering::Relaxed) {
            return Some(Refusal::Budget);
        }
        if let Some(retry_after_secs) = self
            .rate_limits
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(provider)
        {
            return Some(Refusal::RateLimited {
                retry_after_secs: *retry_after_secs,
            });
        }
        None
    }

    /// Whether the run is over, and why. This is the aggregate: the shared budget, or any provider's
    /// `429`. Used for the run report; callers deciding whether to ask a specific provider should
    /// use [`Self::refused_for`].
    pub fn refused(&self) -> Option<Refusal> {
        if self.is_spent.load(Ordering::Relaxed) {
            return Some(Refusal::Budget);
        }
        if let Some((_, retry_after_secs)) = self
            .rate_limits
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .next()
        {
            return Some(Refusal::RateLimited {
                retry_after_secs: *retry_after_secs,
            });
        }
        None
    }

    /// `provider` asked us to stop. Sticky for that provider, and never cached as a fact.
    pub fn rate_limited(&self, provider: &str, retry_after_secs: Option<u64>) {
        self.rate_limits
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(provider.to_string())
            .or_insert(retry_after_secs);
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
        assert_eq!(allowance.take_for("test", 1).await, Ok(()));
        assert_eq!(allowance.take_for("test", 1).await, Ok(()));
        assert_eq!(allowance.take_for("test", 1).await, Err(Refusal::Budget));
        assert_eq!(allowance.refused(), Some(Refusal::Budget));
    }

    /// A batch is charged whole: what it costs is decided before it may run, not one request at a
    /// time. A batch that does not fit is refused before any of it is spent.
    #[tokio::test]
    async fn a_batch_is_charged_whole() {
        let allowance = Allowance::new(3, Duration::ZERO);

        assert_eq!(
            allowance.take_for("test", 3).await,
            Ok(()),
            "exactly affordable"
        );
        assert_eq!(allowance.take_for("test", 1).await, Err(Refusal::Budget));
    }

    #[tokio::test]
    async fn a_batch_that_costs_more_than_the_budget_can_never_run() {
        let allowance = Allowance::new(1, Duration::ZERO);

        assert_eq!(
            allowance.take_for("test", 2).await,
            Err(Refusal::Budget),
            "one left, two needed"
        );
        assert_eq!(allowance.refused(), Some(Refusal::Budget));
    }

    /// A 429 is an instruction for that provider, not a fact about one call.
    #[tokio::test]
    async fn a_rate_limit_refuses_every_later_call_to_that_provider() {
        let allowance = Allowance::new(10, Duration::ZERO);
        allowance.rate_limited("wikidata", Some(30));

        assert_eq!(
            allowance.take_for("wikidata", 1).await,
            Err(Refusal::RateLimited {
                retry_after_secs: Some(30)
            })
        );
        assert_eq!(
            allowance.refused_for("wikidata"),
            Some(Refusal::RateLimited {
                retry_after_secs: Some(30)
            })
        );
        assert_eq!(allowance.refused_for("imdb"), None);
        assert_eq!(allowance.take_for("imdb", 1).await, Ok(()));
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

    /// A `429` without a `Retry-After` still stops that provider: it refused, it just did not say for
    /// how long. Only the time is unknown, not the refusal.
    #[tokio::test]
    async fn a_rate_limit_without_a_retry_after_still_latches() {
        let allowance = Allowance::new(10, Duration::ZERO);
        allowance.rate_limited("wikidata", None);

        assert_eq!(
            allowance.refused_for("wikidata"),
            Some(Refusal::RateLimited {
                retry_after_secs: None
            })
        );
        assert_eq!(
            allowance.take_for("wikidata", 1).await,
            Err(Refusal::RateLimited {
                retry_after_secs: None
            })
        );
        assert_eq!(allowance.take_for("imdb", 1).await, Ok(()));
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
        allowance.take_for("test", 1).await.expect("the first call");
        assert!(
            start.elapsed() < Duration::from_millis(60),
            "the first call waits for nothing: {:?}",
            start.elapsed()
        );

        allowance
            .take_for("test", 1)
            .await
            .expect("the second call");
        assert!(
            start.elapsed() >= Duration::from_millis(60),
            "the second call waits the floor: {:?}",
            start.elapsed()
        );
    }
}
