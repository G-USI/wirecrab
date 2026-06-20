//! Application: collects endpoints into a [`Runner`].
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
//!    into the inner [`Runner`].
//! 4. Register the wires themselves via [`Application::register_wire`] so that
//!    [`Application::stop`] can stop them. When a wire stops, its receivers'
//!    `receive()` calls return `Err`, breaking the consume loops driven by the
//!    [`Runner`].
//! 5. Call [`Application::stop`] before consuming the application if graceful
//!    shutdown is required (the wires are owned by the [`Application`] and
//!    dropped when it is dropped — without `stop()` they will not get a chance
//!    to release resources cleanly).
//! 6. Call [`Application::into_runner`] to extract the [`Runner`] and `.await`
//!    its [`Runner::run`] inside whatever executor was brought (tokio,
//!    Embassy, …). Cancellation = drop the [`Runner`], or wrap `run()` in a
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
//! register publishers. The wire that owns a publisher's sender may still be
//! registered via [`Application::register_wire`] for stop propagation.

use crate::codec::Codec;
use crate::endpoint::handler::Handler;
use crate::endpoint::subscriber::Subscriber;
use crate::runner::Runner;
use crate::utils::structs::*;
use crate::wire::Lifecycle;

/// Collects endpoints (currently: subscriber consume loops) into a [`Runner`],
/// and tracks the wires that own their receivers so the caller can stop them.
///
/// See the module docs for the intended usage sequence.
pub struct Application {
    runner: Runner,
    wires: Vec<Box<dyn Lifecycle>>,
}

impl Application {
    /// Create an empty `Application`.
    pub fn new() -> Self {
        Self {
            runner: Runner::new(),
            wires: Vec::new(),
        }
    }

    /// Register a subscriber: call [`Subscriber::start`] (which starts the
    /// receiver and consumes the subscriber) and push the returned
    /// consume-loop [`BoxFuture`] into the inner [`Runner`].
    ///
    /// The wire that produced the subscriber's receiver should be registered
    /// separately via [`Application::register_wire`] so that
    /// [`Application::stop`] can break this consume loop.
    pub async fn register_subscriber<C, H>(
        &mut self,
        subscriber: Subscriber<C, H>,
    ) -> Result<(), AnyhowError>
    where
        C: Codec + 'static,
        H: Handler<serde_json::Value> + 'static,
    {
        let fut = subscriber.start().await?;
        self.runner.push(fut);
        Ok(())
    }

    /// Register a wire for stop() propagation.
    ///
    /// When [`Application::stop`] is called, every registered wire's
    /// [`Lifecycle::stop`] is invoked. For wire implementations whose
    /// receivers return `Err` after stop (e.g. a stopped in-memory broker),
    /// this is what breaks the subscriber consume loops driven by the
    /// [`Runner`].
    ///
    /// Takes ownership of `wire`. Callers are expected to pull senders and
    /// receivers out of the wire (`new_sender` / `new_receiver` take `&mut
    /// self`) **before** moving the wire into the `Application`. The returned
    /// `Box<dyn Sender>` / `Box<dyn Receiver>` are owned and do not borrow from
    /// the wire, so they outlive the wire's move into `Application`.
    pub fn register_wire<W: Lifecycle + 'static>(&mut self, wire: W) {
        self.wires.push(Box::new(wire));
    }

    /// Stop every registered wire by calling its [`Lifecycle::stop`].
    ///
    /// Returns the first error encountered; on error, remaining wires may not
    /// be stopped.
    ///
    /// This must be called **before** [`Application::into_runner`] — once the
    /// runner is extracted, the wires (still owned by the `Application` that
    /// was consumed) are no longer reachable for explicit stop.
    pub async fn stop(&mut self) -> Result<(), AnyhowError> {
        for wire in &mut self.wires {
            wire.stop().await?;
        }
        Ok(())
    }

    /// Number of background futures currently held by the inner [`Runner`].
    pub fn len(&self) -> usize {
        self.runner.len()
    }

    /// Returns `true` if no futures have been registered.
    pub fn is_empty(&self) -> bool {
        self.runner.is_empty()
    }

    /// Consume the `Application` and return the [`Runner`] that drives all
    /// registered background futures.
    ///
    /// Call this once at startup and `.await` [`Runner::run`] inside the host
    /// executor. The wires registered via [`Application::register_wire`] stay
    /// owned by the `Application` and are dropped when `self` is consumed
    /// (their [`Lifecycle::stop`] is **not** invoked automatically — call
    /// [`Application::stop`] first for graceful shutdown).
    pub fn into_runner(self) -> Runner {
        self.runner
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
    use crate::wire::{IncomingMessage, Receiver, WireMessage};
    use std::collections::VecDeque;
    use std::format;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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

    /// Mock wire that records whether `stop()` was called.
    struct StopTrackingWire {
        stopped: Arc<AtomicBool>,
    }

    #[async_trait]
    impl Lifecycle for StopTrackingWire {
        async fn start(&mut self) -> Result<(), AnyhowError> {
            Ok(())
        }
        async fn stop(&mut self) -> Result<(), AnyhowError> {
            self.stopped.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    /// A wire whose `stop()` always returns `Err`.
    struct FailingWire;

    #[async_trait]
    impl Lifecycle for FailingWire {
        async fn start(&mut self) -> Result<(), AnyhowError> {
            Ok(())
        }
        async fn stop(&mut self) -> Result<(), AnyhowError> {
            Err(anyhow::anyhow!("wire refused to stop"))
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

    /// After registering a subscriber, the inner `Runner` must be non-empty.
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

        assert!(!app.is_empty(), "runner must be non-empty after register");
        assert_eq!(app.len(), 1);
    }

    /// Multiple subscribers accumulate one future each in the inner `Runner`.
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

    /// `into_runner` consumes the application and returns a `Runner` that
    /// actually drives the registered futures to completion.
    #[tokio::test]
    async fn into_runner_drives_registered_future() {
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

        let runner = app.into_runner();
        assert_eq!(runner.len(), 1);
        runner.run().await.expect("runner completes cleanly");

        assert_eq!(
            processed.load(Ordering::SeqCst),
            3,
            "all three messages should reach the handler"
        );
    }

    /// `stop()` with no wires registered returns `Ok` (no-op).
    #[tokio::test]
    async fn stop_with_no_wires_returns_ok() {
        let mut app = Application::new();
        app.stop().await.expect("stop on empty app is no-op");
    }

    /// `stop()` invokes `Lifecycle::stop` on each registered wire.
    #[tokio::test]
    async fn stop_calls_registered_wire_stop() {
        let mut app = Application::new();
        let stopped = Arc::new(AtomicBool::new(false));
        app.register_wire(StopTrackingWire {
            stopped: stopped.clone(),
        });

        assert!(!stopped.load(Ordering::SeqCst), "wire not yet stopped");

        app.stop().await.expect("stop ok");

        assert!(
            stopped.load(Ordering::SeqCst),
            "wire.stop() must be invoked by app.stop()"
        );
    }

    /// `stop()` stops every registered wire, not just the first.
    #[tokio::test]
    async fn stop_calls_all_registered_wires() {
        let mut app = Application::new();
        let s1 = Arc::new(AtomicBool::new(false));
        let s2 = Arc::new(AtomicBool::new(false));
        let s3 = Arc::new(AtomicBool::new(false));
        app.register_wire(StopTrackingWire {
            stopped: s1.clone(),
        });
        app.register_wire(StopTrackingWire {
            stopped: s2.clone(),
        });
        app.register_wire(StopTrackingWire {
            stopped: s3.clone(),
        });

        app.stop().await.expect("stop ok");

        assert!(s1.load(Ordering::SeqCst), "wire 1 must be stopped");
        assert!(s2.load(Ordering::SeqCst), "wire 2 must be stopped");
        assert!(s3.load(Ordering::SeqCst), "wire 3 must be stopped");
    }

    /// `stop()` propagates the first wire error and short-circuits.
    #[tokio::test]
    async fn stop_propagates_error() {
        let mut app = Application::new();
        app.register_wire(FailingWire);

        let err = app.stop().await.expect_err("stop should fail");
        let msg = format!("{err}");
        assert!(
            msg.contains("wire refused to stop"),
            "unexpected error message: {msg}"
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
}
