use std::collections::{HashMap, VecDeque};
use std::fmt::Debug;
use std::hash::Hash;
use std::sync::Mutex;

/// A thread-safe queue that ensures each unique item is processed exactly once.
///
/// Items enqueued multiple times are deduplicated. Each item progresses through
/// three states: `Pending` → `Reserved` (via `pop_next`) → `Completed` (via `complete`).
///
/// # Example
///
/// ```
/// # use dreamspot::common::OneShotQueue;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueStatus {
    Pending,
    Reserved,
    Completed,
}

struct OneShotQueueInner<T: Clone + Debug + Eq + Hash> {
    pending: VecDeque<T>,
    status_map: HashMap<T, QueueStatus>,
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
        if !inner.status_map.contains_key(&item) {
            inner.status_map.insert(item.clone(), QueueStatus::Pending);
            inner.pending.push_back(item);
            return true;
        }
        false
    }

    pub fn enqueue_iter<'a>(&self, items: impl IntoIterator<Item = &'a T>)
    where
        T: 'a,
    {
        let mut inner = self.inner.lock().unwrap();
        for item in items {
            if !inner.status_map.contains_key(item) {
                inner.status_map.insert(item.clone(), QueueStatus::Pending);
                inner.pending.push_back(item.clone());
            }
        }
    }

    pub fn pop_next(&self) -> Option<T> {
        let mut inner = self.inner.lock().unwrap();
        let next = inner.pending.pop_front()?;
        let status = inner.status_map.get_mut(&next);

        if status.is_none() {
            eprintln!("Inconsistent: {:?} popped but untracked in map", next);
            return None;
        }

        let status = status.unwrap();
        match status {
            QueueStatus::Pending => {
                *status = QueueStatus::Reserved;
                Some(next)
            }
            _ => {
                eprintln!("Inconsistent: {:?} popped but already reserved", next);
                None
            }
        }
    }

    pub fn complete(&self, item: T) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let status = inner.status_map.get_mut(&item);

        if status.is_none() {
            eprintln!("Inconsistent: attempted to complete unknown {:?}", item);
            return false;
        }

        let status = status.unwrap();
        match status {
            QueueStatus::Reserved => {
                *status = QueueStatus::Completed;
                true
            }
            QueueStatus::Pending => {
                eprintln!("Attempted to complete pending item {:?}", item);
                false
            }
            QueueStatus::Completed => {
                eprintln!("Attempted to complete already completed item {:?}", item);
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

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

    fn reserved_contains_all<T: Clone + Debug + Eq + Hash>(
        q: &OneShotQueue<T>,
        items: &[T],
    ) -> bool {
        let inner = q.inner.lock().unwrap();
        items
            .iter()
            .all(|i| inner.status_map.get(i) == Some(&QueueStatus::Reserved))
    }

    fn completed_contains<T: Clone + Debug + Eq + Hash>(q: &OneShotQueue<T>, i: &T) -> bool {
        let inner = q.inner.lock().unwrap();
        inner.status_map.get(i) == Some(&QueueStatus::Completed)
    }

    fn completed_contains_all<T: Clone + Debug + Eq + Hash>(
        q: &OneShotQueue<T>,
        items: &[T],
    ) -> bool {
        let inner = q.inner.lock().unwrap();
        items
            .iter()
            .all(|i| inner.status_map.get(i) == Some(&QueueStatus::Completed))
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

        assert!(reserved_contains_all(&queue, &[1, 2]));
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
        assert!(reserved_contains_all(&queue, &[1, 2]));
    }

    #[test]
    fn test_enqueue_iter_dupes() {
        let queue = OneShotQueue::new();
        queue.enqueue_iter(&[1, 2, 2, 3, 1]);
        assert!(count_for_status(&queue, QueueStatus::Pending) == 3);

        assert_eq!(queue.pop_next(), Some(1));
        assert_eq!(queue.pop_next(), Some(2));
        assert_eq!(queue.pop_next(), Some(3));
        assert_eq!(queue.pop_next(), None);

        assert!(count_for_status(&queue, QueueStatus::Pending) == 0);
        assert!(count_for_status(&queue, QueueStatus::Reserved) == 3);
        assert!(reserved_contains_all(&queue, &[1, 2, 3]));
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
        assert!(reserved_contains_all(&queue, &[1, 2]));
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

        assert!(reserved_contains_all(&queue, &[2]));
        assert!(completed_contains(&queue, &1));
    }

    #[test]
    fn test_enqueue_iter() {
        let queue = OneShotQueue::new();
        queue.enqueue_iter(&[1, 2, 3]);

        assert_eq!(queue.pop_next(), Some(1));
        assert_eq!(queue.pop_next(), Some(2));
        assert_eq!(queue.pop_next(), Some(3));
        assert_eq!(queue.pop_next(), None);

        assert!(reserved_contains_all(&queue, &[1, 2, 3]));
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
        let was_completed = queue.complete(42); // Should print error message but not panic TODO
        assert!(!was_completed);
    }

    #[test]
    fn test_multiple_reserves_and_completes() {
        let queue = OneShotQueue::new();
        queue.enqueue_iter(&[1, 2, 3, 4]);

        let item1 = queue.pop_next().unwrap();
        let item2 = queue.pop_next().unwrap();

        queue.complete(item1);

        let item3 = queue.pop_next().unwrap();
        queue.complete(item2);
        queue.complete(item3);

        assert!(!reserved_contains_all(&queue, &[1, 2, 3]));
        assert!(completed_contains_all(&queue, &[1, 2, 3]));
    }

    #[test]
    fn test_with_string_type() {
        let queue = OneShotQueue::new();
        queue.enqueue("hello".to_string());
        queue.enqueue("world".to_string());

        assert_eq!(queue.pop_next(), Some("hello".to_string()));
        assert_eq!(queue.pop_next(), Some("world".to_string()));

        assert!(reserved_contains_all(
            &queue,
            &["hello".to_string(), "world".to_string()]
        ));

        queue.complete("hello".to_string());
        assert!(!reserved_contains_all(&queue, &["hello".to_string()]));
        assert!(completed_contains_all(&queue, &["hello".to_string()]));
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
        queue.enqueue_iter(&(0..100).collect::<Vec<_>>());

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
        assert!(completed_contains_all(
            &queue,
            &(0..100).collect::<Vec<_>>()
        ));
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
                queue_clone.enqueue_iter(&(start..end).collect::<Vec<_>>());
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
        assert!(completed_contains_all(
            &queue,
            &(0..100).collect::<Vec<_>>()
        ));
    }
}
