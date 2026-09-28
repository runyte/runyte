// SPDX-License-Identifier: MPL-2.0

//! Coalesced line comparisons outside the editor's frame path.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    thread,
};

use crate::{
    diff::{Alignment, align_text},
    git::{RowChange, changed_rows},
    text::Text,
};

pub(super) const ASYNC_DIFF_THRESHOLD: usize = 32 * 1024;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum Key {
    Git(PathBuf),
    Pair(u64),
}

#[derive(Debug)]
pub(super) enum Request {
    Git {
        path: PathBuf,
        base_id: u64,
        revision: u64,
        base: Arc<str>,
        text: Text,
    },
    Pair {
        id: u64,
        revisions: (u64, u64),
        left: Text,
        right: Text,
    },
}

impl Request {
    fn key(&self) -> Key {
        match self {
            Self::Git { path, .. } => Key::Git(path.clone()),
            Self::Pair { id, .. } => Key::Pair(*id),
        }
    }

    fn run(self) -> Result {
        match self {
            Self::Git {
                path,
                base_id,
                revision,
                base,
                text,
            } => Result::Git {
                path,
                base_id,
                revision,
                rows: changed_rows(&base, &text.to_string()),
            },
            Self::Pair {
                id,
                revisions,
                left,
                right,
            } => Result::Pair {
                id,
                revisions,
                alignment: align_text(&left.to_string(), &right.to_string()),
            },
        }
    }
}

#[derive(Debug)]
pub(super) enum Result {
    Git {
        path: PathBuf,
        base_id: u64,
        revision: u64,
        rows: Vec<RowChange>,
    },
    Pair {
        id: u64,
        revisions: (u64, u64),
        alignment: Alignment,
    },
}

impl Result {
    fn key(&self) -> Key {
        match self {
            Self::Git { path, .. } => Key::Git(path.clone()),
            Self::Pair { id, .. } => Key::Pair(*id),
        }
    }
}

#[derive(Debug, Default)]
struct Queue {
    active: HashSet<Key>,
    pending: HashMap<Key, Request>,
    pending_order: VecDeque<Key>,
    completed: HashMap<Key, Result>,
    completed_order: VecDeque<Key>,
    stopped: bool,
}

#[derive(Debug, Default)]
pub(super) struct Worker {
    queue: Option<Arc<(Mutex<Queue>, Condvar)>>,
}

impl Worker {
    pub(super) fn new() -> Self {
        Self::default()
    }

    fn start(&mut self) -> &Arc<(Mutex<Queue>, Condvar)> {
        self.queue.get_or_insert_with(|| {
            let queue = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
            let worker_queue = Arc::clone(&queue);
            thread::Builder::new()
                .name("runyte-diff".into())
                .spawn(move || {
                    loop {
                        let request = {
                            let (lock, wake) = &*worker_queue;
                            let mut state = lock.lock().expect("diff queue lock");
                            while state.pending_order.is_empty() && !state.stopped {
                                state = wake.wait(state).expect("diff queue wait");
                            }
                            if state.stopped {
                                break;
                            }
                            let key = state.pending_order.pop_front().expect("nonempty queue");
                            state.pending.remove(&key).expect("queued request")
                        };
                        let result = request.run();
                        let key = result.key();
                        let (lock, _) = &*worker_queue;
                        let mut state = lock.lock().expect("diff queue lock");
                        if state.stopped {
                            break;
                        }
                        if !state.active.contains(&key) {
                            continue;
                        }
                        if !state.completed.contains_key(&key) {
                            state.completed_order.push_back(key.clone());
                        }
                        state.completed.insert(key, result);
                    }
                })
                .expect("spawn diff worker");
            queue
        })
    }

    pub(super) fn submit(&mut self, request: Request) {
        let queue = self.start();
        let key = request.key();
        let (lock, wake) = &**queue;
        let mut state = lock.lock().expect("diff queue lock");
        state.active.insert(key.clone());
        if !state.pending.contains_key(&key) {
            state.pending_order.push_back(key.clone());
        }
        state.pending.insert(key, request);
        wake.notify_one();
    }

    /// Forget queued and completed work for views that no longer exist. The
    /// worker checks the same set before publishing an in-flight result.
    pub(super) fn retain(
        &self,
        git: impl IntoIterator<Item = PathBuf>,
        pairs: impl IntoIterator<Item = u64>,
    ) {
        let Some(queue) = self.queue.as_ref() else {
            return;
        };
        let active = git
            .into_iter()
            .map(Key::Git)
            .chain(pairs.into_iter().map(Key::Pair))
            .collect::<HashSet<_>>();
        let (lock, _) = &**queue;
        let mut state = lock.lock().expect("diff queue lock");
        state.active = active;
        let active = state.active.clone();
        state.pending.retain(|key, _| active.contains(key));
        state.pending_order.retain(|key| active.contains(key));
        state.completed.retain(|key, _| active.contains(key));
        state.completed_order.retain(|key| active.contains(key));
    }

    pub(super) fn try_recv(&self) -> Option<Result> {
        let queue = self.queue.as_ref()?;
        let (lock, _) = &**queue;
        let mut state = lock.lock().expect("diff queue lock");
        let key = state.completed_order.pop_front()?;
        state.completed.remove(&key)
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(queue) = self.queue.as_ref() {
            let (lock, wake) = &**queue;
            let mut state = lock.lock().expect("diff queue lock");
            state.stopped = true;
            state.pending.clear();
            state.pending_order.clear();
            wake.notify_one();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn median(mut samples: Vec<Duration>) -> Duration {
        samples.sort_unstable();
        samples[samples.len() / 2]
    }

    fn measure(label: &str, make: impl Fn() -> Request) {
        for _ in 0..10 {
            let _ = make().run();
        }
        let mut samples = Vec::new();
        for _ in 0..50 {
            let start = Instant::now();
            let _ = make().run();
            samples.push(start.elapsed());
        }
        eprintln!("{label} worker compute: {:?}", median(samples));
    }

    #[test]
    #[ignore = "manual timing, run with --nocapture"]
    fn large_diff_worker_cost() {
        let base = (0..50_000)
            .map(|row| format!("local function item_{row}(x) return x + {row} end\n"))
            .collect::<String>();
        let base: Arc<str> = Arc::from(base);
        let left = Text::from_str(&format!(" {base}"));
        let right = Text::from_str(&format!("  {base}"));
        measure("gutter", || Request::Git {
            path: PathBuf::from("fixture.txt"),
            base_id: 1,
            revision: 1,
            base: Arc::clone(&base),
            text: left.clone(),
        });
        measure("pair", || Request::Pair {
            id: 1,
            revisions: (1, 2),
            left: left.clone(),
            right: right.clone(),
        });
    }
}
