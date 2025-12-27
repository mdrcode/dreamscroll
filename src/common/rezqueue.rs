use std::collections::{HashSet, VecDeque};
use std::hash::Hash;
use std::sync::Mutex;

pub struct RezQueue<T: Clone + Eq + Hash> {
    inner: Mutex<RezQueueInner<T>>,
}

struct RezQueueInner<T: Clone + Eq + Hash> {
    pending: VecDeque<T>,
    reserved: HashSet<T>,
    completed: HashSet<T>,
}

impl<T: Clone + Eq + Hash> Default for RezQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Clone + Eq + Hash> RezQueue<T> {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(RezQueueInner {
                pending: VecDeque::new(),
                reserved: HashSet::new(),
                completed: HashSet::new(),
            }),
        }
    }

    pub fn enqueue(&self, item: T) {
        let mut inner = self.inner.lock().unwrap();
        inner.pending.push_back(item);
    }

    pub fn enqueue_iter(&self, items: impl IntoIterator<Item = T>) {
        let mut inner = self.inner.lock().unwrap();
        inner.pending.extend(items);
    }

    pub fn pop_next(&self) -> Option<T> {
        let mut inner = self.inner.lock().unwrap();
        let next = inner.pending.pop_front()?;
        inner.reserved.insert(next.clone());
        Some(next)
    }

    pub fn complete(&self, item: T) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let was_reserved = inner.reserved.remove(&item);
        if was_reserved {
            inner.completed.insert(item);
        }
        was_reserved
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn pending_empty<T: Clone + Eq + Hash>(q: &RezQueue<T>) -> bool {
        let inner = q.inner.lock().unwrap();
        inner.pending.is_empty()
    }

    fn reserved_empty<T: Clone + Eq + Hash>(q: &RezQueue<T>) -> bool {
        let inner = q.inner.lock().unwrap();
        inner.reserved.is_empty()
    }

    fn reserved_contains<T: Clone + Eq + Hash>(q: &RezQueue<T>, i: &T) -> bool {
        let inner = q.inner.lock().unwrap();
        inner.reserved.contains(i)
    }

    fn reserved_contains_all<T: Clone + Eq + Hash>(q: &RezQueue<T>, items: &[T]) -> bool {
        let inner = q.inner.lock().unwrap();
        items.iter().all(|i| inner.reserved.contains(i))
    }

    fn completed_contains<T: Clone + Eq + Hash>(q: &RezQueue<T>, i: &T) -> bool {
        let inner = q.inner.lock().unwrap();
        inner.completed.contains(i)
    }

    fn completed_contains_all<T: Clone + Eq + Hash>(q: &RezQueue<T>, items: &[T]) -> bool {
        let inner = q.inner.lock().unwrap();
        items.iter().all(|i| inner.completed.contains(i))
    }

    #[test]
    fn test_new_queue_is_empty() {
        let queue: RezQueue<i32> = RezQueue::new();
        assert!(queue.pop_next().is_none());
    }

    #[test]
    fn test_enqueue_and_reserve() {
        let queue = RezQueue::new();
        queue.enqueue(1);
        queue.enqueue(2);

        assert_eq!(queue.pop_next(), Some(1));
        assert_eq!(queue.pop_next(), Some(2));
        assert_eq!(queue.pop_next(), None);

        assert!(reserved_contains_all(&queue, &[1, 2]));
    }

    #[test]
    fn test_enqueue_iter() {
        let queue = RezQueue::new();
        queue.enqueue_iter(vec![1, 2, 3]);

        assert_eq!(queue.pop_next(), Some(1));
        assert_eq!(queue.pop_next(), Some(2));
        assert_eq!(queue.pop_next(), Some(3));
        assert_eq!(queue.pop_next(), None);

        assert!(reserved_contains_all(&queue, &[1, 2, 3]));
    }

    #[test]
    fn test_complete_reserved_item() {
        let queue = RezQueue::new();
        queue.enqueue(1);

        let item = queue.pop_next().unwrap();
        let was_completed = queue.complete(item);

        assert!(!reserved_contains(&queue, &1));
        assert!(completed_contains(&queue, &1));
        assert!(was_completed);
    }

    #[test]
    fn test_complete_unreserved_item() {
        let queue: RezQueue<i32> = RezQueue::new();
        let was_completed = queue.complete(42); // Should print error message but not panic TODO
        assert!(!was_completed);
    }

    #[test]
    fn test_multiple_reserves_and_completes() {
        let queue = RezQueue::new();
        queue.enqueue_iter(vec![1, 2, 3, 4]);

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
        let queue = RezQueue::new();
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
        let queue: RezQueue<i32> = RezQueue::default();
        assert!(queue.pop_next().is_none());
    }

    #[test]
    fn test_fifo_order() {
        let queue = RezQueue::new();
        for i in 0..10 {
            queue.enqueue(i);
        }

        for i in 0..10 {
            assert_eq!(queue.pop_next(), Some(i));
        }
    }

    #[tokio::test]
    async fn test_concurrent_reserve_and_complete() {
        let queue = Arc::new(RezQueue::new());
        queue.enqueue_iter(0..100);

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

        assert!(pending_empty(&queue));
        assert_eq!(queue.pop_next(), None);
        assert!(reserved_empty(&queue));
        assert!(completed_contains_all(
            &queue,
            &(0..100).collect::<Vec<_>>()
        ));
    }

    #[tokio::test]
    async fn test_concurrent_enqueue_and_reserve() {
        let queue = Arc::new(RezQueue::new());

        // Spawn producer tasks
        let mut producer_handles = vec![];
        for batch in 0..4 {
            let queue_clone = queue.clone();
            let handle = tokio::spawn(async move {
                let start = batch * 25;
                let end = start + 25;
                queue_clone.enqueue_iter(start..end);
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
        assert!(pending_empty(&queue));
        assert_eq!(queue.pop_next(), None);
        assert!(reserved_empty(&queue));
        assert!(completed_contains_all(
            &queue,
            &(0..100).collect::<Vec<_>>()
        ));
    }
}
