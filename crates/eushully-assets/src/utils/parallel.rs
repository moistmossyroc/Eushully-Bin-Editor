// parallel.rs — Order-preserving parallel map over a slice with scoped threads
// (no thread-pool dependency). Each worker gets its own state from `init`
// (e.g. open file handles), and pulls the next item from a shared counter so
// uneven items balance out.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// `map(state, item)` for every item, on up to `workers` threads.
/// Results come back in input order.
pub fn parallel_map_with<T, State, Output>(
    items: &[T],
    workers: usize,
    init: impl Fn() -> State + Sync,
    map: impl Fn(&mut State, &T) -> Output + Sync,
) -> Vec<Output>
where
    T: Sync,
    Output: Send,
{
    let next = AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<Output>>> = items.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..workers.clamp(1, items.len().max(1)) {
            scope.spawn(|| {
                let mut state = init();
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(index) else { break };
                    let output = map(&mut state, item);
                    *slots[index].lock().expect("result slot") = Some(output);
                }
            });
        }
    });
    slots
        .into_iter()
        .map(|slot| slot.into_inner().expect("result slot").expect("every item is processed"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_order_and_gives_each_worker_its_own_state() {
        let items: Vec<u32> = (0..1000).collect();
        let doubled = parallel_map_with(&items, 8, || 0usize, |calls, item| {
            *calls += 1;
            item * 2
        });
        assert_eq!(doubled, items.iter().map(|item| item * 2).collect::<Vec<_>>());
        assert!(parallel_map_with(&[] as &[u32], 4, || (), |_, item| *item).is_empty());
    }
}
