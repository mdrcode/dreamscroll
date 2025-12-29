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
/// enqueue attempts are ignored.
///
/// Note, by design, currently this will grow its internal status map indefinitely and
/// thus "leak" memory over time as new unique items are added. So usage is best suited
/// for scenarios where the set of unique items is bounded. In the future, this will be
/// improved with a time-based eviction strategy to both limit memory usage and allow
/// re-processing of items after a certain duration.
///
/// Currently, the items are Clone'd twice per enqueue for internal storage. This could
/// be optimized in the future by using reference counting or other techniques. But for
/// the present, it is suggested to use small, cheap-to-clone types such as integers
/// (database IDs).
///
/// # Example
///
/// ```
/// use dreamspot::common::OneShotQueue;
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

        assert!(vec![1, 2].iter().all(|i| reserved_contains(&queue, i)));
    }

    #[test]
    fn test_enqueue_dupes() {
        let queue = OneShotQueue::new();
        assert!(queue.enqueue(1));
        assert!(!queue.enqueue(1)); // duplicate
        assert!(queue.enqueue(2));
        assert!(count_for_status(&queue, QueueStatus::Pending) == 2);

        assert_eq!(queue.pop_next(), Some(1));
        assert_eq!(queue.pop_next(), Some(2));
        assert_eq!(queue.pop_next(), None);

        assert!(count_for_status(&queue, QueueStatus::Pending) == 0);
        assert!(count_for_status(&queue, QueueStatus::Reserved) == 2);
        assert!(vec![1, 2].iter().all(|i| reserved_contains(&queue, i)));
    }

    #[test]
    fn test_enqueue_iter_dupes() {
        let queue = OneShotQueue::new();
        queue.enqueue_iter([1, 2, 2, 3, 1]);
        assert!(count_for_status(&queue, QueueStatus::Pending) == 3);

        assert_eq!(queue.pop_next(), Some(1));
        assert_eq!(queue.pop_next(), Some(2));
        assert_eq!(queue.pop_next(), Some(3));
        assert_eq!(queue.pop_next(), None);

        assert!(count_for_status(&queue, QueueStatus::Pending) == 0);
        assert!(count_for_status(&queue, QueueStatus::Reserved) == 3);
        assert!(vec![1, 2, 3].iter().all(|i| reserved_contains(&queue, i)));
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

        assert!(count_for_status(&queue, QueueStatus::Pending) == 0);
        assert!(count_for_status(&queue, QueueStatus::Reserved) == 2);
        assert!(vec![1, 2].iter().all(|i| reserved_contains(&queue, i)));
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
    fn test_enqueue_iter() {
        let queue = OneShotQueue::new();
        queue.enqueue_iter([1, 2, 3]);

        assert_eq!(queue.pop_next(), Some(1));
        assert_eq!(queue.pop_next(), Some(2));
        assert_eq!(queue.pop_next(), Some(3));
        assert_eq!(queue.pop_next(), None);

        assert!(vec![1, 2, 3].iter().all(|i| reserved_contains(&queue, i)));
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
    fn test_multiple_reserves_and_completes() {
        let queue = OneShotQueue::new();
        queue.enqueue_iter([1, 2, 3, 4]);

        let item1 = queue.pop_next().unwrap();
        let item2 = queue.pop_next().unwrap();

        queue.complete(item1);

        let item3 = queue.pop_next().unwrap();
        queue.complete(item2);
        queue.complete(item3);

        assert!(!vec![1, 2, 3].iter().all(|i| reserved_contains(&queue, i)));
        assert!(vec![1, 2, 3].iter().all(|i| completed_contains(&queue, i)));
    }

    #[test]
    fn test_with_string_type() {
        let queue = OneShotQueue::new();
        queue.enqueue("hello");
        queue.enqueue("world");

        assert_eq!(queue.pop_next(), Some("hello"));
        assert_eq!(queue.pop_next(), Some("world"));

        assert!(
            vec!["hello", "world"]
                .iter()
                .all(|i| reserved_contains(&queue, &i))
        );

        queue.complete("hello");
        assert!(!reserved_contains(&queue, &"hello"));
        assert!(completed_contains(&queue, &"hello"));
    }

    #[test]
    fn test_default_trait() {
        let queue: OneShotQueue<i32> = OneShotQueue::default();
        assert!(queue.pop_next().is_none());
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
        queue.enqueue_iter((0..100).collect::<Vec<_>>());

        let mut handles = vec![];

        // Spawn multiple workers that reserve and complete items
        for _ in 0..4 {
            let queue_clone = queue.clone();
            let handle = tokio::spawn(async move {
                while let Some(item) = queue_clone.pop_next() {
                    // Simulate some work
                    tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
                    queue_clone.complete(item);
                }
            });
            handles.push(handle);
        }

        // Wait for all workers to complete
        for handle in handles {
            handle.await.unwrap();
        }

        assert_eq!(queue.pop_next(), None);
        assert!(count_for_status(&queue, QueueStatus::Pending) == 0);
        assert!(count_for_status(&queue, QueueStatus::Reserved) == 0);

        assert!(
            (0..100)
                .collect::<Vec<_>>()
                .iter()
                .all(|i| completed_contains(&queue, &i))
        );
    }

    #[tokio::test]
    async fn test_concurrent_enqueue_and_reserve() {
        let queue = Arc::new(OneShotQueue::new());

        // Spawn producer tasks
        let mut producer_handles = vec![];
        for batch in 0..4 {
            let queue_clone = queue.clone();
            let handle = tokio::spawn(async move {
                let start = batch * 25;
                let end = start + 25;
                queue_clone.enqueue_iter((start..end).collect::<Vec<_>>());
            });
            producer_handles.push(handle);
        }

        // Wait for producers to finish
        for handle in producer_handles {
            handle.await.unwrap();
        }

        // Spawn consumer tasks
        let mut consumer_handles = vec![];
        for _ in 0..3 {
            let queue_clone = queue.clone();
            let handle = tokio::spawn(async move {
                let mut reserved_items = vec![];
                while let Some(item) = queue_clone.pop_next() {
                    reserved_items.push(item);
                }
                for item in reserved_items {
                    queue_clone.complete(item);
                }
            });
            consumer_handles.push(handle);
        }

        // Wait for all consumers to complete
        for handle in consumer_handles {
            handle.await.unwrap();
        }

        // Verify all 100 items were processed
        assert_eq!(queue.pop_next(), None);
        assert!(count_for_status(&queue, QueueStatus::Pending) == 0);
        assert!(count_for_status(&queue, QueueStatus::Reserved) == 0);
        assert!(
            (0..100)
                .collect::<Vec<_>>()
                .iter()
                .all(|i| completed_contains(&queue, &i))
        );
    }
}
