//! Application: collects subscriber consume-loop futures and drives them
//! concurrently via `futures_util::try_join_all`.
//!
//! An [`Application`] is the top-level entry point for wiring up an AsyncAPI
//! runtime without spawning. The intended usage sequence is:
//!
//! 1. Create an [`Application`].
//! 2. Build wires (e.g. an in-memory wire) and pull senders / receivers out of
//!    them via `Wire::new_sender` / `Wire::new_receiver` (both take `&mut
//!    self`).
//! 3. Build [`Subscriber`]s from the receivers and call
//!    [`Application::register_subscriber`] to push their consume-loop futures
//!    into the application.
//! 4. Call [`Application::run`] to drive all registered futures concurrently.
//!    `run` consumes `self` and `.await`s `try_join_all` over the collected
//!    futures inside whatever executor was brought (tokio, Embassy, …).
//!    Cancellation = drop the future returned by `run`, or wrap it in a
//!    `select!` at the call site.
//!
//! The `Application` is **not** generic over codec / handler types:
//! [`Subscriber::start`] returns a type-erased [`BoxFuture`], so a single
//! `Application` can hold an arbitrary mix of subscribers.
//!
//! # Publishers
//!
//! [`Publisher`](crate::endpoint::publisher::Publisher) endpoints do not have a
//! background loop — they are held by the caller and used imperatively
//! (`publisher.send(payload).await`). The `Application` therefore does not
//! register publishers. The wire that owns a publisher's sender is managed
//! (started / stopped) by the caller alongside the application.
//!
//! [`Subscriber::start`]: crate::endpoint::subscriber::Subscriber::start
//! [`BoxFuture`]: crate::future::BoxFuture

use crate::codec::Codec;
use crate::endpoint::handler::Handler;
use crate::endpoint::subscriber::Subscriber;
use crate::future::BoxFuture;
use crate::utils::structs::*;
use futures_util::future::try_join_all;

/// Collects subscriber consume-loop futures and drives them concurrently.
///
/// Holds a `Vec<BoxFuture>` returned by [`Subscriber::start`]. Call
/// [`Application::run`] to drive them all via `try_join_all`.
///
/// See the module docs for the intended usage sequence.
///
/// [`Subscriber::start`]: crate::endpoint::subscriber::Subscriber::start
pub struct Application {
    futures: Vec<BoxFuture>,
}

impl Application {
    /// Create an empty `Application`.
    pub fn new() -> Self {
        Self {
            futures: Vec::new(),
        }
    }

    /// Register a subscriber: call [`Subscriber::start`] (which starts the
    /// receiver and consumes the subscriber) and push the returned
    /// consume-loop [`BoxFuture`] into the application.
    ///
    /// The wire that produced the subscriber's receiver is managed (started /
    /// stopped) by the caller alongside the application — stopping the wire
    /// causes its receivers' `receive()` calls to return `Err`, which breaks
    /// the consume loop.
    ///
    /// [`BoxFuture`]: crate::future::BoxFuture
    pub async fn register_subscriber<C, H>(
        &mut self,
        subscriber: Subscriber<C, H>,
    ) -> Result<(), AnyhowError>
    where
        C: Codec + 'static,
        H: Handler<serde_json::Value> + 'static,
    {
        let fut = subscriber.start().await?;
        self.futures.push(fut);
        Ok(())
    }

    /// Number of background futures currently held.
    pub fn len(&self) -> usize {
        self.futures.len()
    }

    /// Returns `true` if no futures have been registered.
    pub fn is_empty(&self) -> bool {
        self.futures.is_empty()
    }

    /// Consume the `Application` and drive all registered futures concurrently
    /// via `try_join_all`.
    ///
    /// Returns `Ok(())` when every future completes successfully, or the first
    /// error encountered (remaining futures are dropped, i.e. cancelled).
    ///
    /// The returned future is `Send` (suitable for `tokio::spawn(app.run())`).
    /// Cancellation = drop the returned future, or wrap it in a `select!` at
    /// the call site.
    pub async fn run(self) -> Result<(), AnyhowError> {
        try_join_all(self.futures).await?;
        Ok(())
    }
}

impl Default for Application {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::endpoint::handler::{HandlerError, MessageContext};
    use crate::wire::{IncomingMessage, Lifecycle, Receiver, WireMessage};
    use std::collections::VecDeque;
    use std::format;
    use std::future::Future;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    /// Pass-through JSON codec used to round-trip `serde_json::Value` payloads.
    /// Mirrors the `JsonCodec` pattern from `subscriber::tests`.
    struct JsonCodec;

    impl Codec for JsonCodec {
        fn encode<D>(&self, value: &D) -> Result<Vec<u8>, AnyhowError>
        where
            D: serde::ser::Serialize + ?Sized,
        {
            serde_json::to_vec(value).map_err(Into::into)
        }

        fn decode<D>(&self, bytes: &[u8]) -> Result<D, AnyhowError>
        where
            D: serde::de::DeserializeOwned,
        {
            serde_json::from_slice(bytes).map_err(Into::into)
        }

        fn extract_field(&self, _bytes: &[u8], _location: &str) -> Result<String, AnyhowError> {
            Err(anyhow::anyhow!("JsonCodec::extract_field not implemented"))
        }
    }

    /// Mock receiver that drains a shared queue of [`WireMessage`]s.
    ///
    /// When the queue is empty and `stopped` is true, [`Receiver::receive`]
    /// returns `Err` to simulate a wire shutdown (the loop then exits cleanly).
    /// Mirrors the `MockReceiver` pattern from `subscriber::tests`.
    struct MockReceiver {
        queue: Arc<Mutex<VecDeque<WireMessage>>>,
        stopped: bool,
    }

    impl MockReceiver {
        fn new(queue: Arc<Mutex<VecDeque<WireMessage>>>, stopped: bool) -> Self {
            Self { queue, stopped }
        }
    }

    #[async_trait]
    impl Lifecycle for MockReceiver {
        async fn start(&mut self) -> Result<(), AnyhowError> {
            Ok(())
        }
        async fn stop(&mut self) -> Result<(), AnyhowError> {
            Ok(())
        }
    }

    #[async_trait]
    impl Receiver for MockReceiver {
        async fn receive(&mut self) -> Result<IncomingMessage, AnyhowError> {
            // Drop the guard before any await — MutexGuard is !Send and would
            // break the `Send` bound on the returned future.
            let msg = { self.queue.lock().expect("queue lock").pop_front() };
            if let Some(msg) = msg {
                Ok(IncomingMessage::from_wire(msg))
            } else if self.stopped {
                Err(anyhow::anyhow!("wire stopped"))
            } else {
                // Empty queue, not stopped: would block forever. Never hit in
                // the current tests because `stopped` is always `true`.
                std::future::pending::<()>().await;
                unreachable!()
            }
        }
    }

    /// Build a `WireMessage` whose payload is the given bytes.
    fn make_msg(payload: &[u8]) -> WireMessage {
        WireMessage {
            headers: BTreeMap::new(),
            payload: payload.to_vec(),
            correlation_id: None,
            content_type: None,
            reply_to: None,
        }
    }

    #[test]
    fn empty_application() {
        let app = Application::new();
        assert!(app.is_empty());
        assert_eq!(app.len(), 0);
    }

    #[test]
    fn default_is_empty() {
        let app = Application::default();
        assert!(app.is_empty());
        assert_eq!(app.len(), 0);
    }

    /// After registering a subscriber, the application must be non-empty.
    #[tokio::test]
    async fn register_subscriber_adds_future() {
        let mut app = Application::new();
        assert!(app.is_empty());

        let queue = Arc::new(Mutex::new(VecDeque::from([make_msg(b"1")])));
        let receiver = Box::new(MockReceiver::new(queue, true));
        let handler = |_payload: serde_json::Value, _ctx: &MessageContext| async move {
            Ok::<(), HandlerError>(())
        };

        let subscriber = Subscriber::new(JsonCodec, handler, receiver);
        app.register_subscriber(subscriber)
            .await
            .expect("register ok");

        assert!(!app.is_empty(), "app must be non-empty after register");
        assert_eq!(app.len(), 1);
    }

    /// Multiple subscribers accumulate one future each.
    #[tokio::test]
    async fn register_multiple_subscribers_accumulates_futures() {
        let mut app = Application::new();

        for _ in 0..3 {
            let queue = Arc::new(Mutex::new(VecDeque::from([make_msg(b"1")])));
            let receiver = Box::new(MockReceiver::new(queue, true));
            let handler = |_payload: serde_json::Value, _ctx: &MessageContext| async move {
                Ok::<(), HandlerError>(())
            };
            let subscriber = Subscriber::new(JsonCodec, handler, receiver);
            app.register_subscriber(subscriber)
                .await
                .expect("register ok");
        }

        assert_eq!(app.len(), 3, "three subscribers → three futures");
    }

    /// `Application::run` drives all registered futures via `try_join_all`.
    #[tokio::test]
    async fn run_drives_registered_futures() {
        let mut app = Application::new();

        let processed = Arc::new(AtomicUsize::new(0));
        let queue = Arc::new(Mutex::new(VecDeque::from([
            make_msg(b"1"),
            make_msg(b"2"),
            make_msg(b"3"),
        ])));
        let receiver = Box::new(MockReceiver::new(queue, true));
        let processed_handler = processed.clone();
        let handler = move |_payload: serde_json::Value, _ctx: &MessageContext| {
            let p = processed_handler.clone();
            async move {
                p.fetch_add(1, Ordering::SeqCst);
                Ok::<(), HandlerError>(())
            }
        };

        let subscriber = Subscriber::new(JsonCodec, handler, receiver);
        app.register_subscriber(subscriber)
            .await
            .expect("register ok");

        app.run().await.expect("run completes cleanly");

        assert_eq!(
            processed.load(Ordering::SeqCst),
            3,
            "all three messages should reach the handler"
        );
    }

    /// Sanity: the `Application` is `Send` (required for `#[tokio::test]`
    /// multi-thread runtimes and for the futures returned by its async methods
    /// to be `Send`). This is a compile-time check; the assert is never run.
    #[test]
    fn application_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<Application>();
    }

    /// The future returned by `run()` must be `Send` so that callers can
    /// `tokio::spawn(app.run())`. Compile-time check only — never run.
    #[test]
    fn run_future_is_send() {
        fn assert_send_future<F: Future + Send>(_f: F) {}
        let app = Application::new();
        assert_send_future(app.run());
    }

    /// `try_join_all` short-circuits on the first `Err`: pushing one `Ok`
    /// future and one `Err` future must cause `run().await` to return `Err`.
    ///
    /// This exercises the fail-fast cancellation path of `try_join_all`. The
    /// current subscriber consume loops never return `Err` (they map all
    /// `receive()` errors to `Ok(())`), so this test keeps the short-circuit
    /// path live-tested for forward compatibility.
    #[tokio::test]
    async fn try_join_all_cancels_on_error() {
        let mut app = Application::new();
        app.futures.push(Box::pin(async { Ok(()) }));
        app.futures
            .push(Box::pin(async { Err(anyhow::anyhow!("future exploded")) }));

        let err = app.run().await.expect_err("run should fail");
        assert!(
            format!("{err}").contains("future exploded"),
            "expected error message, got: {err}"
        );
    }
}
