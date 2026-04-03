use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use tokio::sync::{Mutex, Notify};
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct PostHydrationTitleScanQueueState {
    queued: VecDeque<String>,
    queued_or_running: HashSet<String>,
}

#[derive(Clone, Default)]
pub struct PostHydrationTitleScanQueue {
    state: Arc<Mutex<PostHydrationTitleScanQueueState>>,
    wake: Arc<Notify>,
}

impl PostHydrationTitleScanQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn enqueue(&self, title_id: String) -> bool {
        let should_wake = {
            let mut state = self.state.lock().await;
            if !state.queued_or_running.insert(title_id.clone()) {
                return false;
            }
            state.queued.push_back(title_id);
            true
        };

        if should_wake {
            self.wake.notify_one();
        }

        true
    }

    pub async fn dequeue(&self, token: &CancellationToken) -> Option<String> {
        loop {
            if let Some(title_id) = {
                let mut state = self.state.lock().await;
                state.queued.pop_front()
            } {
                return Some(title_id);
            }

            tokio::select! {
                _ = token.cancelled() => return None,
                _ = self.wake.notified() => {}
            }
        }
    }

    pub async fn finish(&self, title_id: &str) {
        let mut state = self.state.lock().await;
        state.queued_or_running.remove(title_id);
    }
}
