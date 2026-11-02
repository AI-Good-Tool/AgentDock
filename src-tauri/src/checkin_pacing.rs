//! Shared pacing for manual and scheduled check-ins in this process.
use std::sync::Arc;
use tokio::{
    sync::{Mutex, OwnedMutexGuard},
    time::{sleep, Duration, Instant},
};

#[derive(Default)]
pub struct Pacer {
    finished: Arc<Mutex<Option<Instant>>>,
}

impl Pacer {
    pub async fn acquire(&self, interval: Duration) -> Guard {
        let finished = self.finished.clone().lock_owned().await;
        if let Some(at) = *finished {
            sleep(interval.saturating_sub(at.elapsed())).await;
        }
        Guard { finished }
    }

    /// Only call from a blocking worker; its guard outlives async cancellation.
    pub fn acquire_blocking(&self, interval: Duration) -> Guard {
        let finished = self.finished.clone().blocking_lock_owned();
        if let Some(at) = *finished {
            std::thread::sleep(interval.saturating_sub(at.elapsed()));
        }
        Guard { finished }
    }
}

pub struct Guard {
    finished: OwnedMutexGuard<Option<Instant>>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        // Failures and cancellation receive the same cooldown as success.
        *self.finished = Some(Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocking_worker_waits_after_completion() {
        let pacer = Pacer::default();
        let first = pacer.acquire_blocking(Duration::ZERO);
        let start = Instant::now();
        drop(first);
        let interval = Duration::from_millis(10);
        drop(pacer.acquire_blocking(interval));
        assert!(start.elapsed() >= interval);
    }

    #[tokio::test]
    async fn cancelling_caller_does_not_release_a_running_blocking_worker() {
        let pacer = Arc::new(Pacer::default());
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let caller_pacer = pacer.clone();
        let caller = tokio::spawn(async move {
            tokio::task::spawn_blocking(move || {
                let _pacing = caller_pacer.acquire_blocking(Duration::ZERO);
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            })
            .await
            .unwrap();
        });
        started_rx.await.unwrap();
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());
        let next = pacer.acquire(Duration::ZERO);
        tokio::pin!(next);
        let blocked = futures_util::poll!(&mut next).is_pending();
        release_tx.send(()).unwrap();
        assert!(
            blocked,
            "the worker must retain the queue after its caller is cancelled"
        );
        drop(next.await);
    }

    #[tokio::test(start_paused = true)]
    async fn waits_after_completion_and_serializes_competing_triggers() {
        let pacer = Pacer::default();
        let interval = Duration::from_secs(3);
        let start = Instant::now();
        let first = pacer.acquire(interval).await;
        assert_eq!(
            start.elapsed(),
            Duration::ZERO,
            "first account starts immediately"
        );
        let next = pacer.acquire(interval);
        tokio::pin!(next);
        assert!(futures_util::poll!(&mut next).is_pending());
        tokio::time::advance(Duration::from_secs(10)).await;
        assert!(
            futures_util::poll!(&mut next).is_pending(),
            "no overlap with a running account"
        );
        drop(first);
        let finished = Instant::now();
        let second = next.await;
        assert_eq!(
            finished.elapsed(),
            interval,
            "delay starts after completion"
        );
        drop(second);
    }

    #[tokio::test(start_paused = true)]
    async fn honors_custom_zero_and_elapsed_intervals() {
        let pacer = Pacer::default();
        drop(pacer.acquire(Duration::ZERO).await);
        let start = Instant::now();
        drop(pacer.acquire(Duration::from_secs(7)).await);
        assert_eq!(start.elapsed(), Duration::from_secs(7));
        let start = Instant::now();
        drop(pacer.acquire(Duration::ZERO).await);
        assert_eq!(start.elapsed(), Duration::ZERO);
        tokio::time::advance(Duration::from_secs(5)).await;
        let start = Instant::now();
        drop(pacer.acquire(Duration::from_secs(3)).await);
        assert_eq!(
            start.elapsed(),
            Duration::ZERO,
            "an elapsed cooldown adds no delay"
        );
    }
}
