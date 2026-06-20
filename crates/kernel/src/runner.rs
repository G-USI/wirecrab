use crate::utils::structs::*;

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

/// Type-erased future for background tasks.
///
/// Represents a `Future` that outputs `Result<(), AnyhowError>` and is
/// `Send + 'static`. Owned by the [`Runner`] which drives it to completion.
pub type BoxFuture = Pin<Box<dyn Future<Output = Result<(), AnyhowError>> + Send + 'static>>;

/// Collects background futures (consume loops, dispatchers) for the caller to drive.
///
/// The caller `.await`s [`Runner::run`] inside whatever executor they brought
/// (tokio, Embassy, etc.). This is the **no-spawn pattern** — the library returns
/// futures, never spawns them. Cancellation = drop the [`Runner`] (wrap in
/// `select!` at the call site).
///
/// Based on the aimdb Runner pattern, validated for cross-runtime portability.
///
/// # Cooperative polling
///
/// [`Runner::run`] polls all collected futures in round-robin order using a
/// safe `core::future::poll_fn` based loop (no `unsafe`, no `futures` crate
/// dependency). When no future makes progress, it yields to the host executor
/// via a one-shot yield future, avoiding a busy-loop.
pub struct Runner {
    futures: Vec<BoxFuture>,
}

impl Runner {
    /// Create an empty `Runner`.
    pub fn new() -> Self {
        Self {
            futures: Vec::new(),
        }
    }

    /// Push a background future to be driven by [`Runner::run`].
    pub fn push(&mut self, fut: BoxFuture) {
        self.futures.push(fut);
    }

    /// Returns `true` if no futures have been pushed.
    pub fn is_empty(&self) -> bool {
        self.futures.is_empty()
    }

    /// Returns the number of pending futures.
    pub fn len(&self) -> usize {
        self.futures.len()
    }

    /// Drive all futures to completion.
    ///
    /// Returns `Ok(())` when every future completes successfully, or the first
    /// error encountered. On error, remaining futures are dropped (cancelled).
    ///
    /// Cancellation is cooperative: drop the `Runner` (or wrap `run()` in a
    /// `select!` at the call site) to cancel all pending work.
    pub async fn run(mut self) -> Result<(), AnyhowError> {
        while !self.futures.is_empty() {
            let outcome = core::future::poll_fn(|cx| Self::poll_round(&mut self.futures, cx)).await;

            match outcome {
                RoundOutcome::Done => return Ok(()),
                RoundOutcome::Progress => continue,
                RoundOutcome::Stalled => YieldOnce::new().await,
                RoundOutcome::Err(e) => return Err(e),
            }
        }

        Ok(())
    }

    fn poll_round(futures: &mut Vec<BoxFuture>, cx: &mut Context<'_>) -> Poll<RoundOutcome> {
        let mut progress = false;

        for i in (0..futures.len()).rev() {
            match futures[i].as_mut().poll(cx) {
                Poll::Ready(Ok(())) => {
                    drop(futures.swap_remove(i));
                    progress = true;
                }
                Poll::Ready(Err(e)) => return Poll::Ready(RoundOutcome::Err(e)),
                Poll::Pending => {}
            }
        }

        let result = if futures.is_empty() {
            RoundOutcome::Done
        } else if progress {
            RoundOutcome::Progress
        } else {
            RoundOutcome::Stalled
        };
        Poll::Ready(result)
    }
}

impl Default for Runner {
    fn default() -> Self {
        Self::new()
    }
}

enum RoundOutcome {
    Done,
    Progress,
    Stalled,
    Err(AnyhowError),
}

/// A future that yields exactly once to the executor, then completes.
///
/// Used to break up a tight poll loop when no background future is ready,
/// giving the host executor a chance to run other tasks.
struct YieldOnce {
    yielded: bool,
}

impl YieldOnce {
    fn new() -> Self {
        Self { yielded: false }
    }
}

impl Future for YieldOnce {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.yielded {
            Poll::Ready(())
        } else {
            self.yielded = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::format;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn boxed<F>(fut: F) -> BoxFuture
    where
        F: Future<Output = Result<(), AnyhowError>> + Send + 'static,
    {
        Box::pin(fut)
    }

    #[tokio::test]
    async fn empty_runner_returns_ok() {
        let runner = Runner::new();
        let result = runner.run().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn default_runner_is_empty() {
        let runner = Runner::default();
        assert!(runner.is_empty());
        assert_eq!(runner.len(), 0);
        assert!(runner.run().await.is_ok());
    }

    #[tokio::test]
    async fn single_future_completes() {
        let flag = Arc::new(AtomicBool::new(false));
        let flag_setter = flag.clone();

        let mut runner = Runner::new();
        runner.push(boxed(async move {
            flag_setter.store(true, Ordering::SeqCst);
            Ok(())
        }));

        assert_eq!(runner.len(), 1);
        assert!(!runner.is_empty());

        runner.run().await.unwrap();
        assert!(flag.load(Ordering::SeqCst), "flag should be set after run");
    }

    #[tokio::test]
    async fn multiple_futures_all_complete() {
        let counter = Arc::new(AtomicUsize::new(0));

        let mut runner = Runner::new();
        for _ in 0..3 {
            let c = counter.clone();
            runner.push(boxed(async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }));
        }

        assert_eq!(runner.len(), 3);
        runner.run().await.unwrap();
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn error_propagates_from_failing_future() {
        let mut runner = Runner::new();
        runner.push(boxed(async { Ok(()) }));
        runner.push(boxed(async { Err(anyhow::anyhow!("future exploded")) }));
        runner.push(boxed(async { Ok(()) }));

        let err = runner.run().await.unwrap_err();
        assert!(
            format!("{err}").contains("future exploded"),
            "expected error message, got: {err}"
        );
    }

    #[tokio::test]
    async fn futures_with_yields_complete() {
        let counter = Arc::new(AtomicUsize::new(0));

        let mut runner = Runner::new();
        for _ in 0..5 {
            let c = counter.clone();
            runner.push(boxed(async move {
                YieldOnce::new().await;
                c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }));
        }

        runner.run().await.unwrap();
        assert_eq!(counter.load(Ordering::SeqCst), 5);
    }

    #[test]
    fn is_empty_and_len_track_pushes() {
        let mut runner = Runner::new();
        assert!(runner.is_empty());
        assert_eq!(runner.len(), 0);

        runner.push(boxed(async { Ok(()) }));
        assert!(!runner.is_empty());
        assert_eq!(runner.len(), 1);

        runner.push(boxed(async { Ok(()) }));
        assert_eq!(runner.len(), 2);
    }
}
