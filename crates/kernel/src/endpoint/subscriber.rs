//! Subscriber endpoint: a consume loop that decodes wire messages and
//! dispatches the decoded payload to a [`Handler`].
//!
//! The loop is returned as a [`BoxFuture`] from [`Subscriber::start`]; the
//! caller drives it (e.g. by pushing it into a [`crate::runner::Runner`]).
//! The loop never spawns its own task — this is the no-spawn pattern.
//!
//! # Error isolation (W13 fix)
//!
//! Handler errors do **not** break the loop:
//! - [`HandlerError::Transient`] → the message is nacked and the loop continues.
//! - [`HandlerError::Reject`]   → the message is rejected and the loop continues.
//!
//! The only way the loop exits normally is when the wire reports an error on
//! [`crate::wire::Receiver::receive`] (interpreted as "wire stopped").

use crate::codec::Codec;
use crate::endpoint::handler::{Handler, HandlerError, MessageContext};
use crate::runner::BoxFuture;
use crate::utils::structs::*;
use crate::wire::Receiver;

/// A subscriber endpoint that consumes messages from a wire, decodes them via
/// a [`Codec`], and dispatches the decoded payload to a [`Handler`].
///
/// `C` is the codec used to decode the raw `payload` bytes into a
/// `serde_json::Value` (the MVP payload type). `H` is the handler that
/// processes each decoded payload.
pub struct Subscriber<C: Codec, H> {
    codec: C,
    handler: H,
    receiver: Box<dyn Receiver>,
}

impl<C, H> Subscriber<C, H>
where
    C: Codec + 'static,
    H: Handler<serde_json::Value> + 'static,
{
    /// Create a new subscriber from its three components.
    pub fn new(codec: C, handler: H, receiver: Box<dyn Receiver>) -> Self {
        Self {
            codec,
            handler,
            receiver,
        }
    }

    /// Start the wire, then return the consume loop as a [`BoxFuture`].
    ///
    /// This consumes `self` so that the returned future can own the receiver,
    /// codec, and handler by move (the future must be `'static`).
    ///
    /// Returns `Err` if [`Receiver::start`] fails; in that case the receiver
    /// is dropped.
    ///
    /// # Consume loop semantics
    ///
    /// For each received message:
    /// 1. Decode the payload bytes via `codec.decode::<serde_json::Value>`.
    ///    On decode failure → nack the message and continue.
    /// 2. Call `handler.handle(payload, ctx)`.
    /// 3. Ack/nack/reject based on the handler result:
    ///    - `Ok(())` → ack
    ///    - `Err(HandlerError::Transient(_))` → nack, **loop continues**
    ///    - `Err(HandlerError::Reject(_))` → reject, **loop continues**
    ///
    /// The loop exits cleanly with `Ok(())` when `receiver.receive()` returns
    /// an error (wire stopped).
    pub async fn start(mut self) -> Result<BoxFuture, AnyhowError> {
        self.receiver.start().await?;

        let mut receiver = self.receiver;
        let codec = self.codec;
        let handler = self.handler;

        Ok(Box::pin(async move {
            while let Ok(mut msg) = receiver.receive().await {
                let payload = match codec.decode::<serde_json::Value>(&msg.payload) {
                    Ok(v) => v,
                    Err(_) => {
                        // Undecodable payload: nack and keep looping.
                        msg.nack();
                        continue;
                    }
                };
                let ctx = MessageContext {
                    headers: msg.headers.clone(),
                    correlation_id: msg.correlation_id.clone(),
                    reply_to: msg.reply_to.clone(),
                    content_type: msg.content_type.clone(),
                };
                match handler.handle(payload, &ctx).await {
                    Ok(()) => msg.ack(),
                    Err(HandlerError::Transient(_)) => msg.nack(),
                    Err(HandlerError::Reject(_)) => msg.reject(),
                }
            }
            Ok(())
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{IncomingMessage, Lifecycle, WireMessage};
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::vec;

    /// A pass-through JSON codec used to round-trip `serde_json::Value` payloads.
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
    /// When `stopped` is true and the queue is empty, [`Receiver::receive`]
    /// returns `Err` to simulate a wire shutdown (the loop then exits cleanly).
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
            let msg = { self.queue.lock().expect("queue lock").pop_front() };
            if let Some(msg) = msg {
                Ok(IncomingMessage::from_wire(msg))
            } else if self.stopped {
                Err(anyhow::anyhow!("wire stopped"))
            } else {
                // Queue empty but not stopped: block forever (never hit in
                // current tests because `stopped` is always `true`).
                std::future::pending::<()>().await;
                unreachable!()
            }
        }
    }

    /// Build a `WireMessage` whose payload is `payload` (already JSON-encoded bytes).
    fn make_msg(payload: &[u8]) -> WireMessage {
        WireMessage {
            headers: BTreeMap::new(),
            payload: payload.to_vec(),
            correlation_id: None,
            content_type: None,
            reply_to: None,
        }
    }

    /// Drain-driven: 3 messages, handler returns Ok for each.
    /// The loop must process all 3 and exit cleanly when the wire "stops"
    /// (empty queue + `stopped`).
    #[tokio::test]
    async fn subscribe_acks_on_success() {
        let queue = Arc::new(Mutex::new(VecDeque::from([
            make_msg(b"1"),
            make_msg(b"2"),
            make_msg(b"3"),
        ])));
        let processed = Arc::new(AtomicUsize::new(0));

        let receiver = Box::new(MockReceiver::new(queue.clone(), true));
        let processed_handler = processed.clone();
        let handler = move |_payload: serde_json::Value, _ctx: &MessageContext| {
            let p = processed_handler.clone();
            async move {
                p.fetch_add(1, Ordering::SeqCst);
                Ok::<(), HandlerError>(())
            }
        };

        let subscriber = Subscriber::new(JsonCodec, handler, receiver);
        let fut = subscriber.start().await.expect("receiver.start() ok");
        fut.await.expect("loop should complete cleanly");

        assert_eq!(
            processed.load(Ordering::SeqCst),
            3,
            "all three messages should reach the handler"
        );
    }

    /// W13 fix proof: handler returns Transient on msg 2, but msg 3 must
    /// still be processed. If the loop broke on Transient, msg 3 would
    /// never run.
    #[tokio::test]
    async fn error_isolation_handler_error_continues() {
        let queue = Arc::new(Mutex::new(VecDeque::from([
            make_msg(b"1"),
            make_msg(b"2"),
            make_msg(b"3"),
        ])));
        let seen: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::new()));

        let receiver = Box::new(MockReceiver::new(queue.clone(), true));
        let seen_handler = seen.clone();
        let handler = move |payload: serde_json::Value, _ctx: &MessageContext| {
            let s = seen_handler.clone();
            async move {
                let n = payload.as_u64().unwrap_or(0);
                s.lock().expect("seen lock").push(n);
                if n == 2 {
                    return Err::<(), HandlerError>(HandlerError::Transient(anyhow::anyhow!(
                        "transient blip on msg 2"
                    )));
                }
                Ok(())
            }
        };

        let subscriber = Subscriber::new(JsonCodec, handler, receiver);
        let fut = subscriber.start().await.expect("receiver.start() ok");
        fut.await.expect("loop should complete cleanly");

        let observed = seen.lock().expect("seen lock").clone();
        assert_eq!(
            observed,
            vec![1, 2, 3],
            "transient error on msg 2 must NOT stop the loop"
        );
    }

    /// Handler Reject on msg 1, msg 2 must still be processed.
    #[tokio::test]
    async fn reject_continues_loop() {
        let queue = Arc::new(Mutex::new(VecDeque::from([make_msg(b"1"), make_msg(b"2")])));
        let seen: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::new()));

        let receiver = Box::new(MockReceiver::new(queue.clone(), true));
        let seen_handler = seen.clone();
        let handler = move |payload: serde_json::Value, _ctx: &MessageContext| {
            let s = seen_handler.clone();
            async move {
                let n = payload.as_u64().unwrap_or(0);
                s.lock().expect("seen lock").push(n);
                if n == 1 {
                    return Err::<(), HandlerError>(HandlerError::Reject(anyhow::anyhow!(
                        "reject msg 1"
                    )));
                }
                Ok(())
            }
        };

        let subscriber = Subscriber::new(JsonCodec, handler, receiver);
        let fut = subscriber.start().await.expect("receiver.start() ok");
        fut.await.expect("loop should complete cleanly");

        let observed = seen.lock().expect("seen lock").clone();
        assert_eq!(
            observed,
            vec![1, 2],
            "reject on msg 1 must NOT stop the loop"
        );
    }

    /// Receiver returns Err → loop breaks and the future returns Ok(()).
    #[tokio::test]
    async fn loop_exits_on_wire_error() {
        let queue: Arc<Mutex<VecDeque<WireMessage>>> = Arc::new(Mutex::new(VecDeque::new()));
        let receiver = Box::new(MockReceiver::new(queue.clone(), true));
        let handler = |_payload: serde_json::Value, _ctx: &MessageContext| async move {
            Ok::<(), HandlerError>(())
        };

        let subscriber = Subscriber::new(JsonCodec, handler, receiver);
        let fut = subscriber.start().await.expect("receiver.start() ok");
        let result = fut.await;
        assert!(
            result.is_ok(),
            "loop should exit cleanly with Ok(()) on wire error, got: {result:?}"
        );
    }

    /// Decode failure on a malformed payload: loop nacks the message and keeps
    /// going. The next valid message must still be processed.
    #[tokio::test]
    async fn decode_failure_nacks_and_continues() {
        let queue = Arc::new(Mutex::new(VecDeque::from([
            make_msg(b"{ not valid json"), // invalid → nack + continue
            make_msg(b"42"),               // valid → handler invoked
        ])));
        let processed = Arc::new(AtomicUsize::new(0));

        let receiver = Box::new(MockReceiver::new(queue.clone(), true));
        let processed_handler = processed.clone();
        let handler = move |_payload: serde_json::Value, _ctx: &MessageContext| {
            let p = processed_handler.clone();
            async move {
                p.fetch_add(1, Ordering::SeqCst);
                Ok::<(), HandlerError>(())
            }
        };

        let subscriber = Subscriber::new(JsonCodec, handler, receiver);
        let fut = subscriber.start().await.expect("receiver.start() ok");
        fut.await.expect("loop should complete cleanly");

        assert_eq!(
            processed.load(Ordering::SeqCst),
            1,
            "only the valid message should reach the handler"
        );
    }

    /// Verify `Subscriber::new` wires its three components correctly by
    /// observing that `start()` invokes `receiver.start()`.
    #[tokio::test]
    async fn new_invokes_receiver_start() {
        let queue: Arc<Mutex<VecDeque<WireMessage>>> = Arc::new(Mutex::new(VecDeque::new()));
        let receiver = Box::new(MockReceiver::new(queue.clone(), true));
        let handler = |_payload: serde_json::Value, _ctx: &MessageContext| async move {
            Ok::<(), HandlerError>(())
        };

        let subscriber = Subscriber::new(JsonCodec, handler, receiver);
        let fut = subscriber.start().await.expect("receiver.start() ok");
        fut.await.expect("loop completes");
    }
}
