use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use std::fmt::Debug;
use std::hash::Hash;
use std::sync::Mutex;

/// A thread-safe queue that ensures each unique item is processed exactly once.
///
/// If the caller attempts to enqueue an item multiple times, the request is ignored.
/// Each item progresses through three states: `Pending` → `Reserved` (via `pop_next`)
/// → `Completed` (via `complete`). If an item is already `Reserved` or `Completed`,
/// attempts to enqueue it again are ignored.
///
/// Note, by design, currently this will grow its internal status map indefinitely and
/// thus "leak" memory over time as new unique items are added. So usage is best suited
/// for scenarios where the set of unique items is bounded. In the future, this will be
/// improved with a time-based eviction strategy to both limit memory usage and allow
/// re-processing of items after a certain expiration.
///
/// Currently, the items are consumed and clone'd per enqueue for a total of 2x for
/// internal storage. This could be optimized in the future by using reference counting
/// or other techniques. But for the present, it is suggested to use small,
/// cheap-to-clone types such as integers (database IDs).
///
/// # Example
///
/// ```
/// use dreamscroll::common::OneShotQueue;
///
/// let queue = OneShotQueue::new();
/// queue.enqueue(1);
/// queue.enqueue(1); // Duplicate, ignored
///
/// if let Some(item) = queue.pop_next() {
///     // Process item...
///     queue.complete(item);
/// }
///
/// queue.enqueue(1); // Still ignored, already completed
/// ```
pub struct OneShotQueue<T: Clone + Debug + Eq + Hash> {
    inner: Mutex<OneShotQueueInner<T>>,
}

struct OneShotQueueInner<T: Clone + Debug + Eq + Hash> {
    pending: VecDeque<T>,
    status_map: HashMap<T, QueueStatus>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueStatus {
    Pending,
    Reserved,
    Completed,
}

impl<T: Clone + Debug + Eq + Hash> Default for OneShotQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Clone + Debug + Eq + Hash> OneShotQueue<T> {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(OneShotQueueInner {
                pending: VecDeque::new(),
                status_map: HashMap::new(),
            }),
        }
    }

    pub fn enqueue(&self, item: T) -> bool {
        let mut inner = self.inner.lock().unwrap();
        if let Entry::Vacant(e) = inner.status_map.entry(item.clone()) {
            e.insert(QueueStatus::Pending);
            inner.pending.push_back(item);
            return true;
        }
        false
    }

    pub fn enqueue_iter(&self, items: impl IntoIterator<Item = T>) -> usize {
        let mut count = 0;
        let mut inner = self.inner.lock().unwrap();
        for item in items {
            if let Entry::Vacant(e) = inner.status_map.entry(item.clone()) {
                e.insert(QueueStatus::Pending);
                inner.pending.push_back(item);
                count += 1;
            }
        }
        count
    }

    pub fn pop_next(&self) -> Option<T> {
        let mut inner = self.inner.lock().unwrap();
        let next = inner.pending.pop_front()?;
        let status = inner
            .status_map
            .get_mut(&next)
            .expect("invariant violated: pending item not in status_map");

        match status {
            QueueStatus::Pending => {
                *status = QueueStatus::Reserved;
                Some(next)
            }
            QueueStatus::Reserved | QueueStatus::Completed => {
                unreachable!(
                    "invariant violated: only pending items should be in pending queue, got {:?} for {:?}",
                    status, next
                );
            }
        }
    }

    pub fn complete(&self, item: T) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let status = inner.status_map.get_mut(&item);

        if status.is_none() {
            tracing::warn!("Attempted to complete unknown, untracked item {:?}", item);
            return false;
        }

        let status = status.unwrap();
        match status {
            QueueStatus::Reserved => {
                *status = QueueStatus::Completed;
                true
            }
            QueueStatus::Pending => {
                tracing::warn!("Attempted to complete pending item {:?}", item);
                false
            }
            QueueStatus::Completed => {
                tracing::warn!("Attempted to complete already completed item {:?}", item);
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, vec};

    use super::*;

    fn count_for_status<T: Clone + Debug + Eq + Hash>(
        q: &OneShotQueue<T>,
        s: QueueStatus,
    ) -> usize {
        let inner = q.inner.lock().unwrap();
        inner.status_map.values().filter(|stat| **stat == s).count()
    }

    fn reserved_contains<T: Clone + Debug + Eq + Hash>(q: &OneShotQueue<T>, i: &T) -> bool {
        let inner = q.inner.lock().unwrap();
        inner.status_map.get(i) == Some(&QueueStatus::Reserved)
    }

    fn completed_contains<T: Clone + Debug + Eq + Hash>(q: &OneShotQueue<T>, i: &T) -> bool {
        let inner = q.inner.lock().unwrap();
        inner.status_map.get(i) == Some(&QueueStatus::Completed)
    }

    /// The queue is empty and every item in `range` reached `Completed`.
    fn assert_drained_completed(q: &OneShotQueue<i32>, range: std::ops::Range<i32>) {
        assert_eq!(q.pop_next(), None);
        assert_eq!(count_for_status(q, QueueStatus::Pending), 0);
        assert_eq!(count_for_status(q, QueueStatus::Reserved), 0);
        assert!(range.clone().all(|i| completed_contains(q, &i)), "{range:?}");
    }

    #[test]
    fn test_new_queue_is_empty() {
        let queue: OneShotQueue<i32> = OneShotQueue::new();
        assert!(queue.pop_next().is_none());
    }

    #[test]
    fn test_enqueue_and_pop() {
        let queue = OneShotQueue::new();
        queue.enqueue(1);
        queue.enqueue(2);

        assert_eq!(queue.pop_next(), Some(1));
        assert_eq!(queue.pop_next(), Some(2));
        assert_eq!(queue.pop_next(), None);

        assert!([1, 2].iter().all(|i| reserved_contains(&queue, i)));
    }

    #[test]
    fn test_enqueue_dupes() {
        let queue = OneShotQueue::new();
        assert!(queue.enqueue(1));
        assert!(!queue.enqueue(1)); // duplicate
        assert!(queue.enqueue(2));

        assert_eq!(queue.pop_next(), Some(1));
        assert_eq!(queue.pop_next(), Some(2));
        assert_eq!(queue.pop_next(), None);

        assert!([1, 2].iter().all(|i| reserved_contains(&queue, i)));
    }

    #[test]
    fn test_enqueue_iter_dupes() {
        let queue = OneShotQueue::new();
        // Only the two duplicates are refused; the count reports newly queued items.
        assert_eq!(queue.enqueue_iter([1, 2, 2, 3, 1]), 3);

        assert_eq!(queue.pop_next(), Some(1));
        assert_eq!(queue.pop_next(), Some(2));
        assert_eq!(queue.pop_next(), Some(3));
        assert_eq!(queue.pop_next(), None);

        assert!([1, 2, 3].iter().all(|i| reserved_contains(&queue, i)));
    }

    #[test]
    fn test_enqueue_dupes_after_pop() {
        let queue = OneShotQueue::new();
        assert!(queue.enqueue(1));
        assert_eq!(queue.pop_next(), Some(1));
        assert!(!queue.enqueue(1)); // duplicate after pop
        assert!(queue.enqueue(2));

        assert_eq!(queue.pop_next(), Some(2));
        assert_eq!(queue.pop_next(), None);

        assert!([1, 2].iter().all(|i| reserved_contains(&queue, i)));
    }

    #[test]
    fn test_enqueue_dupes_after_complete() {
        let queue = OneShotQueue::new();
        assert!(queue.enqueue(1));
        let item = queue.pop_next().unwrap();
        assert!(queue.complete(item));
        assert!(!queue.enqueue(1)); // duplicate after complete
        assert!(queue.enqueue(2));

        assert_eq!(queue.pop_next(), Some(2));
        assert_eq!(queue.pop_next(), None);

        assert!(reserved_contains(&queue, &2));
        assert!(completed_contains(&queue, &1));
    }

    #[test]
    fn test_complete_reserved_item() {
        let queue = OneShotQueue::new();
        queue.enqueue(1);

        let item = queue.pop_next().unwrap();
        let was_completed = queue.complete(item);

        assert!(!reserved_contains(&queue, &1));
        assert!(completed_contains(&queue, &1));
        assert!(was_completed);
    }

    #[test]
    fn test_complete_unreserved_item() {
        let queue: OneShotQueue<i32> = OneShotQueue::new();
        let was_completed = queue.complete(42);
        assert!(!was_completed);
    }

    #[test]
    fn test_complete_pending_item_is_refused_and_still_deliverable() {
        let queue = OneShotQueue::new();
        assert!(queue.enqueue(7));

        // Completing something no worker reserved must not consume it.
        assert!(!queue.complete(7));
        assert_eq!(queue.pop_next(), Some(7));
    }

    #[test]
    fn test_multiple_reserves_and_completes() {
        let queue = OneShotQueue::new();
        queue.enqueue_iter([1, 2, 3, 4]);

        let item1 = queue.pop_next().unwrap();
        let item2 = queue.pop_next().unwrap();

        queue.complete(item1);

        let item3 = queue.pop_next().unwrap();
        queue.complete(item2);
        queue.complete(item3);

        // Completed items stay tracked and refuse re-enqueue; the fourth was
        // never popped, so it is still deliverable.
        assert!([1, 2, 3].iter().all(|i| completed_contains(&queue, i)));
        assert!([1, 2, 3].iter().all(|i| !reserved_contains(&queue, i)));
        assert!(!queue.enqueue(2));
        assert_eq!(queue.pop_next(), Some(4));
    }

    #[test]
    fn test_fifo_order() {
        let queue = OneShotQueue::new();
        for i in 0..10 {
            queue.enqueue(i);
        }

        for i in 0..10 {
            assert_eq!(queue.pop_next(), Some(i));
        }
    }

    #[tokio::test]
    async fn test_concurrent_reserve_and_complete() {
        let queue = Arc::new(OneShotQueue::new());
        queue.enqueue_iter(0..100);

        let mut handles = vec![];

        // Workers reserve an item, yield so the others can reserve concurrently,
        // then complete it: an item must never be handed to two workers.
        for _ in 0..4 {
            let queue_clone = queue.clone();
            handles.push(tokio::spawn(async move {
                while let Some(item) = queue_clone.pop_next() {
                    tokio::task::yield_now().await;
                    queue_clone.complete(item);
                }
            }));
        }

        for handle in handles {
            handle.await.unwrap();
        }

        assert_drained_completed(&queue, 0..100);
    }

    #[tokio::test]
    async fn test_concurrent_enqueue_and_reserve() {
        let queue = Arc::new(OneShotQueue::new());

        // Producers overlap: each batch re-sends the tail of the previous one, so
        // a duplicate racing a first insert must still be refused exactly once.
        let mut producer_handles = vec![];
        for batch in 0..4 {
            let queue_clone = queue.clone();
            producer_handles.push(tokio::spawn(async move {
                let start = batch * 24;
                queue_clone.enqueue_iter(start..start + 28);
            }));
        }
        for handle in producer_handles {
            handle.await.unwrap();
        }

        // Consumers hold their reserved items while the others pop.
        let mut consumer_handles = vec![];
        for _ in 0..3 {
            let queue_clone = queue.clone();
            consumer_handles.push(tokio::spawn(async move {
                let mut reserved_items = vec![];
                while let Some(item) = queue_clone.pop_next() {
                    reserved_items.push(item);
                }
                for item in reserved_items {
                    queue_clone.complete(item);
                }
            }));
        }
        for handle in consumer_handles {
            handle.await.unwrap();
        }

        assert_drained_completed(&queue, 0..100);
    }
}
