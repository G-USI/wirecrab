#![forbid(unsafe_code)]
//! In-memory wire implementation for testing and local development.
//!
//! Delivery modes:
//! - [`DeliveryMode::WorkQueue`]: competing consumers (shared queue, first-to-poll wins).
//! - [`DeliveryMode::PubSub`]: fan-out (each consumer gets its own copy).
//!
//! No persistence, no redelivery, no wildcard routing. Each [`InMemoryWire`]
//! owns its own [`InMemoryBus`] via `Arc`.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use anyhow::anyhow;
use wirecrab_kernel::prelude::{AnyhowError, async_trait};
use wirecrab_kernel::wire::{
    ChannelConfig, DeliveryMode, IncomingMessage, Lifecycle, Receiver, Sender, Wire, WireMessage,
};

// ---------------------------------------------------------------------------
// AsyncNotify — minimal single-permit notifier (no tokio "sync" feature needed)
// ---------------------------------------------------------------------------

struct NotifyState {
    flag: bool,
    waker: Option<Waker>,
}

pub struct AsyncNotify {
    state: Mutex<NotifyState>,
}

impl AsyncNotify {
    fn new() -> Self {
        Self {
            state: Mutex::new(NotifyState {
                flag: false,
                waker: None,
            }),
        }
    }

    pub fn notify_one(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.flag = true;
            if let Some(waker) = state.waker.take() {
                waker.wake();
            }
        }
    }

    fn notified(&self) -> Notified<'_> {
        Notified { state: &self.state }
    }
}

struct Notified<'a> {
    state: &'a Mutex<NotifyState>,
}

impl Future for Notified<'_> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Ok(mut state) = self.state.lock() {
            if state.flag {
                state.flag = false;
                state.waker = None;
                return Poll::Ready(());
            }
            state.waker = Some(cx.waker().clone());
        }
        Poll::Pending
    }
}

// ---------------------------------------------------------------------------
// InMemoryBus — shared message store
// ---------------------------------------------------------------------------

type WorkQueue = VecDeque<IncomingMessage>;
type PubSubQueues = Vec<VecDeque<IncomingMessage>>;

/// Shared in-memory message bus backing all producers and consumers created
/// from a single [`InMemoryWire`].
///
/// Locks are held only across trivial push/pop operations (no `.await` while
/// holding a lock), so `std::sync::Mutex` is appropriate.
#[derive(Default)]
pub struct InMemoryBus {
    work_queues: Mutex<HashMap<String, WorkQueue>>,
    work_notifies: Mutex<HashMap<String, Arc<AsyncNotify>>>,
    pubsub_queues: Mutex<HashMap<String, PubSubQueues>>,
    pubsub_notifies: Mutex<HashMap<String, Vec<Arc<AsyncNotify>>>>,
    next_consumer_id: Mutex<HashMap<String, usize>>,
}

impl InMemoryBus {
    /// Create a new empty bus.
    pub fn new() -> Self {
        Self::default()
    }

    /// Publish a message to the bus.
    ///
    /// - `WorkQueue`: push to the shared deque and notify one waiting consumer.
    /// - `PubSub`: push to every consumer's deque and notify every waiting consumer.
    pub fn publish(&self, address: &str, msg: WireMessage, mode: DeliveryMode) {
        let incoming = IncomingMessage::from_wire(msg);
        match mode {
            DeliveryMode::WorkQueue => {
                if let Ok(mut queues) = self.work_queues.lock() {
                    queues
                        .entry(address.to_string())
                        .or_default()
                        .push_back(incoming);
                }
                if let Ok(notifies) = self.work_notifies.lock() {
                    if let Some(notify) = notifies.get(address) {
                        notify.notify_one();
                    }
                }
            }
            DeliveryMode::PubSub => {
                if let Ok(mut queues) = self.pubsub_queues.lock() {
                    if let Some(consumer_queues) = queues.get_mut(address) {
                        for q in consumer_queues.iter_mut() {
                            q.push_back(incoming.clone());
                        }
                    }
                }
                if let Ok(notifies) = self.pubsub_notifies.lock() {
                    if let Some(consumer_notifies) = notifies.get(address) {
                        for notify in consumer_notifies {
                            notify.notify_one();
                        }
                    }
                }
            }
        }
    }

    /// Pop a message from the shared work queue (WorkQueue mode).
    pub fn get_work_message(&self, address: &str) -> Option<IncomingMessage> {
        self.work_queues
            .lock()
            .ok()
            .and_then(|mut queues| queues.get_mut(address).and_then(|q| q.pop_front()))
    }

    /// Pop a message from a specific consumer's pubsub queue.
    pub fn get_pubsub_message(
        &self,
        address: &str,
        consumer_id: usize,
    ) -> Option<IncomingMessage> {
        self.pubsub_queues
            .lock()
            .ok()
            .and_then(|mut queues| {
                queues
                    .get_mut(address)
                    .and_then(|q| q.get_mut(consumer_id).and_then(|q| q.pop_front()))
            })
    }

    /// Get or create the `AsyncNotify` for a WorkQueue address.
    pub fn get_work_notify(&self, address: &str) -> Arc<AsyncNotify> {
        let mut notifies = self
            .work_notifies
            .lock()
            .expect("work_notifies mutex poisoned");
        notifies
            .entry(address.to_string())
            .or_insert_with(|| Arc::new(AsyncNotify::new()))
            .clone()
    }

    /// Register a new PubSub consumer for an address.
    ///
    /// Returns `(consumer_id, notify_handle)`. Each registered consumer gets
    /// its own deque and notify, enabling true fan-out.
    pub fn register_pubsub_consumer(&self, address: &str) -> (usize, Arc<AsyncNotify>) {
        let consumer_id = {
            let mut ids = self
                .next_consumer_id
                .lock()
                .expect("next_consumer_id mutex poisoned");
            let id = ids.entry(address.to_string()).or_insert(0);
            let current = *id;
            *id += 1;
            current
        };
        {
            let mut queues = self
                .pubsub_queues
                .lock()
                .expect("pubsub_queues mutex poisoned");
            queues
                .entry(address.to_string())
                .or_default()
                .push(VecDeque::new());
        }
        let notify = {
            let mut notifies = self
                .pubsub_notifies
                .lock()
                .expect("pubsub_notifies mutex poisoned");
            let vec = notifies.entry(address.to_string()).or_default();
            while vec.len() <= consumer_id {
                vec.push(Arc::new(AsyncNotify::new()));
            }
            vec[consumer_id].clone()
        };
        (consumer_id, notify)
    }
}

// ---------------------------------------------------------------------------
// InMemoryProducer — implements Sender
// ---------------------------------------------------------------------------

/// Producer endpoint backed by an [`InMemoryBus`].
pub struct InMemoryProducer {
    bus: Arc<InMemoryBus>,
    address: String,
    delivery: DeliveryMode,
    started: bool,
}

impl InMemoryProducer {
    /// Create a new producer for the given address and delivery mode.
    pub fn new(bus: Arc<InMemoryBus>, address: String, delivery: DeliveryMode) -> Self {
        Self {
            bus,
            address,
            delivery,
            started: false,
        }
    }
}

#[async_trait]
impl Lifecycle for InMemoryProducer {
    async fn start(&mut self) -> Result<(), AnyhowError> {
        self.started = true;
        Ok(())
    }
    async fn stop(&mut self) -> Result<(), AnyhowError> {
        self.started = false;
        Ok(())
    }
}

#[async_trait]
impl Sender for InMemoryProducer {
    async fn send_batch(
        &mut self,
        messages: &[WireMessage],
        address_override: Option<&str>,
    ) -> Result<(), AnyhowError> {
        if !self.started {
            return Err(anyhow!("producer not started"));
        }
        let addr = address_override.unwrap_or(&self.address);
        for msg in messages {
            self.bus.publish(addr, msg.clone(), self.delivery);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// InMemoryConsumer — implements Receiver
// ---------------------------------------------------------------------------

/// Consumer endpoint backed by an [`InMemoryBus`].
pub struct InMemoryConsumer {
    bus: Arc<InMemoryBus>,
    address: String,
    delivery: DeliveryMode,
    started: bool,
    notify: Option<Arc<AsyncNotify>>,
    consumer_id: Option<usize>,
}

impl InMemoryConsumer {
    /// Create a new consumer for the given address and delivery mode.
    pub fn new(bus: Arc<InMemoryBus>, address: String, delivery: DeliveryMode) -> Self {
        Self {
            bus,
            address,
            delivery,
            started: false,
            notify: None,
            consumer_id: None,
        }
    }
}

#[async_trait]
impl Lifecycle for InMemoryConsumer {
    async fn start(&mut self) -> Result<(), AnyhowError> {
        match self.delivery {
            DeliveryMode::WorkQueue => {
                self.notify = Some(self.bus.get_work_notify(&self.address));
            }
            DeliveryMode::PubSub => {
                let (id, notify) = self.bus.register_pubsub_consumer(&self.address);
                self.consumer_id = Some(id);
                self.notify = Some(notify);
            }
        }
        self.started = true;
        Ok(())
    }
    async fn stop(&mut self) -> Result<(), AnyhowError> {
        self.started = false;
        if let Some(notify) = &self.notify {
            notify.notify_one();
        }
        Ok(())
    }
}

#[async_trait]
impl Receiver for InMemoryConsumer {
    async fn receive(&mut self) -> Result<IncomingMessage, AnyhowError> {
        if !self.started {
            return Err(anyhow!("consumer not started"));
        }
        loop {
            let msg = match self.delivery {
                DeliveryMode::WorkQueue => self.bus.get_work_message(&self.address),
                DeliveryMode::PubSub => {
                    let id = self.consumer_id.unwrap_or(0);
                    self.bus.get_pubsub_message(&self.address, id)
                }
            };
            if let Some(m) = msg {
                return Ok(m);
            }
            if let Some(notify) = &self.notify {
                notify.notified().await;
            }
            if !self.started {
                return Err(anyhow!("consumer stopped"));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// InMemoryWire — implements Wire
// ---------------------------------------------------------------------------

/// In-memory [`Wire`] implementation. Owns an [`InMemoryBus`] via `Arc` and
/// hands out clones of that `Arc` to every producer and consumer it creates.
pub struct InMemoryWire {
    bus: Arc<InMemoryBus>,
    started: bool,
}

impl InMemoryWire {
    /// Create a new in-memory wire with a fresh bus.
    pub fn new() -> Self {
        Self {
            bus: Arc::new(InMemoryBus::new()),
            started: false,
        }
    }

    /// Get a clone of the bus `Arc` (useful for tests / direct inspection).
    pub fn bus(&self) -> Arc<InMemoryBus> {
        self.bus.clone()
    }
}

impl Default for InMemoryWire {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Lifecycle for InMemoryWire {
    async fn start(&mut self) -> Result<(), AnyhowError> {
        self.started = true;
        Ok(())
    }
    async fn stop(&mut self) -> Result<(), AnyhowError> {
        self.started = false;
        Ok(())
    }
}

#[async_trait]
impl Wire for InMemoryWire {
    async fn new_sender(&mut self, config: &ChannelConfig) -> Result<Box<dyn Sender>, AnyhowError> {
        Ok(Box::new(InMemoryProducer::new(
            self.bus.clone(),
            config.address.clone(),
            config.delivery,
        )))
    }
    async fn new_receiver(
        &mut self,
        config: &ChannelConfig,
    ) -> Result<Box<dyn Receiver>, AnyhowError> {
        Ok(Box::new(InMemoryConsumer::new(
            self.bus.clone(),
            config.address.clone(),
            config.delivery,
        )))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use wirecrab_kernel::wire::WireMessage;

    fn make_msg(payload: &[u8]) -> WireMessage {
        WireMessage {
            headers: BTreeMap::new(),
            payload: payload.to_vec(),
            correlation_id: None,
            content_type: None,
            reply_to: None,
        }
    }

    async fn setup(
        address: &str,
        delivery: DeliveryMode,
    ) -> (InMemoryWire, Box<dyn Sender>, Box<dyn Receiver>) {
        let mut wire = InMemoryWire::new();
        wire.start().await.unwrap();
        let cfg = ChannelConfig::new(address.to_string(), delivery);
        let mut producer = wire.new_sender(&cfg).await.unwrap();
        let mut consumer = wire.new_receiver(&cfg).await.unwrap();
        producer.start().await.unwrap();
        consumer.start().await.unwrap();
        (wire, producer, consumer)
    }

    async fn yield_n(n: usize) {
        for _ in 0..n {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn send_and_receive_single() {
        let (_wire, mut producer, mut consumer) = setup("q1", DeliveryMode::WorkQueue).await;

        producer
            .send_batch(&[make_msg(b"hello")], None)
            .await
            .unwrap();
        let msg = consumer.receive().await.unwrap();

        assert_eq!(msg.payload, b"hello");
    }

    #[tokio::test]
    async fn send_batch_fifo_order() {
        let (_wire, mut producer, mut consumer) = setup("fifo", DeliveryMode::WorkQueue).await;

        producer
            .send_batch(&[make_msg(b"a"), make_msg(b"b"), make_msg(b"c")], None)
            .await
            .unwrap();

        let m1 = consumer.receive().await.unwrap();
        let m2 = consumer.receive().await.unwrap();
        let m3 = consumer.receive().await.unwrap();

        assert_eq!(m1.payload, b"a");
        assert_eq!(m2.payload, b"b");
        assert_eq!(m3.payload, b"c");
    }

    #[tokio::test]
    async fn competing_consumers() {
        let mut wire = InMemoryWire::new();
        wire.start().await.unwrap();
        let cfg = ChannelConfig::new("competing".to_string(), DeliveryMode::WorkQueue);
        let mut producer = wire.new_sender(&cfg).await.unwrap();
        let mut c1 = wire.new_receiver(&cfg).await.unwrap();
        let mut c2 = wire.new_receiver(&cfg).await.unwrap();
        producer.start().await.unwrap();
        c1.start().await.unwrap();
        c2.start().await.unwrap();

        producer
            .send_batch(
                &[make_msg(b"1"), make_msg(b"2"), make_msg(b"3"), make_msg(b"4")],
                None,
            )
            .await
            .unwrap();

        let h1 = tokio::spawn(async move {
            let mut got = Vec::new();
            for _ in 0..2 {
                got.push(c1.receive().await.unwrap().payload);
            }
            got
        });
        let h2 = tokio::spawn(async move {
            let mut got = Vec::new();
            for _ in 0..2 {
                got.push(c2.receive().await.unwrap().payload);
            }
            got
        });

        let r1 = h1.await.unwrap();
        let r2 = h2.await.unwrap();

        let mut all: Vec<Vec<u8>> = r1.into_iter().chain(r2.into_iter()).collect();
        all.sort();
        assert_eq!(
            all,
            vec![b"1".to_vec(), b"2".to_vec(), b"3".to_vec(), b"4".to_vec()]
        );
    }

    #[tokio::test]
    async fn address_override_routes_correctly() {
        let mut wire = InMemoryWire::new();
        wire.start().await.unwrap();
        let cfg_a = ChannelConfig::new("a".to_string(), DeliveryMode::WorkQueue);
        let cfg_b = ChannelConfig::new("b".to_string(), DeliveryMode::WorkQueue);
        let mut producer = wire.new_sender(&cfg_a).await.unwrap();
        let mut consumer_b = wire.new_receiver(&cfg_b).await.unwrap();
        producer.start().await.unwrap();
        consumer_b.start().await.unwrap();

        producer
            .send_batch(&[make_msg(b"to-b")], Some("b"))
            .await
            .unwrap();

        let msg = consumer_b.receive().await.unwrap();
        assert_eq!(msg.payload, b"to-b");

        assert!(wire.bus().get_work_message("a").is_none());
    }

    #[tokio::test]
    async fn address_override_default() {
        let (_wire, mut producer, mut consumer) = setup("default", DeliveryMode::WorkQueue).await;

        producer
            .send_batch(&[make_msg(b"ping")], None)
            .await
            .unwrap();

        let msg = consumer.receive().await.unwrap();
        assert_eq!(msg.payload, b"ping");
    }

    #[tokio::test]
    async fn ack_sets_flag() {
        let (_wire, mut producer, mut consumer) = setup("ack", DeliveryMode::WorkQueue).await;

        producer.send_batch(&[make_msg(b"x")], None).await.unwrap();

        let mut msg = consumer.receive().await.unwrap();
        assert!(!msg.is_acknowledged());
        msg.ack();
        assert!(msg.is_acknowledged());
        assert!(!msg.is_nacked());
        assert!(!msg.is_rejected());
    }

    #[tokio::test]
    async fn nack_sets_flag() {
        let (_wire, mut producer, mut consumer) = setup("nack", DeliveryMode::WorkQueue).await;

        producer.send_batch(&[make_msg(b"x")], None).await.unwrap();

        let mut msg = consumer.receive().await.unwrap();
        assert!(!msg.is_nacked());
        msg.nack();
        assert!(msg.is_nacked());
        assert!(!msg.is_acknowledged());
    }

    #[tokio::test]
    async fn reject_sets_flag() {
        let (_wire, mut producer, mut consumer) = setup("reject", DeliveryMode::WorkQueue).await;

        producer.send_batch(&[make_msg(b"x")], None).await.unwrap();

        let mut msg = consumer.receive().await.unwrap();
        assert!(!msg.is_rejected());
        msg.reject();
        assert!(msg.is_rejected());
        assert!(!msg.is_acknowledged());
    }

    #[tokio::test]
    async fn consumer_stop_breaks_receive() {
        let mut wire = InMemoryWire::new();
        wire.start().await.unwrap();
        let cfg = ChannelConfig::new("stop".to_string(), DeliveryMode::WorkQueue);
        let mut consumer = wire.new_receiver(&cfg).await.unwrap();
        consumer.start().await.unwrap();
        consumer.stop().await.unwrap();

        let result = consumer.receive().await;
        assert!(result.is_err(), "receive after stop must error");
    }

    #[tokio::test]
    async fn producer_not_started_errors() {
        let mut wire = InMemoryWire::new();
        wire.start().await.unwrap();
        let cfg = ChannelConfig::new("nostart".to_string(), DeliveryMode::WorkQueue);
        let mut producer = wire.new_sender(&cfg).await.unwrap();
        let result = producer.send_batch(&[make_msg(b"x")], None).await;
        assert!(result.is_err(), "send_batch before start must error");
    }

    #[tokio::test]
    async fn consumer_not_started_errors() {
        let mut wire = InMemoryWire::new();
        wire.start().await.unwrap();
        let cfg = ChannelConfig::new("nostart-rx".to_string(), DeliveryMode::WorkQueue);
        let mut consumer = wire.new_receiver(&cfg).await.unwrap();
        let result = consumer.receive().await;
        assert!(result.is_err(), "receive before start must error");
    }

    #[tokio::test]
    async fn receive_blocks_until_publish_wakes_it() {
        let mut wire = InMemoryWire::new();
        wire.start().await.unwrap();
        let cfg = ChannelConfig::new("wake".to_string(), DeliveryMode::WorkQueue);
        let mut consumer = wire.new_receiver(&cfg).await.unwrap();
        consumer.start().await.unwrap();

        let bus = wire.bus();
        let address = "wake".to_string();
        tokio::spawn(async move {
            yield_n(16).await;
            bus.publish(&address, make_msg(b"wake-up"), DeliveryMode::WorkQueue);
        });

        let msg = consumer.receive().await.unwrap();
        assert_eq!(msg.payload, b"wake-up");
    }

    #[tokio::test]
    async fn pubsub_fanout() {
        let mut wire = InMemoryWire::new();
        wire.start().await.unwrap();
        let cfg = ChannelConfig::new("fanout".to_string(), DeliveryMode::PubSub);
        let mut producer = wire.new_sender(&cfg).await.unwrap();
        let mut c1 = wire.new_receiver(&cfg).await.unwrap();
        let mut c2 = wire.new_receiver(&cfg).await.unwrap();
        producer.start().await.unwrap();
        c1.start().await.unwrap();
        c2.start().await.unwrap();

        producer
            .send_batch(&[make_msg(b"broadcast")], None)
            .await
            .unwrap();

        let m1 = c1.receive().await.unwrap();
        let m2 = c2.receive().await.unwrap();

        assert_eq!(m1.payload, b"broadcast");
        assert_eq!(m2.payload, b"broadcast");
    }

    #[tokio::test]
    async fn pubsub_independent_addresses() {
        let mut wire = InMemoryWire::new();
        wire.start().await.unwrap();
        let cfg_a = ChannelConfig::new("pa".to_string(), DeliveryMode::PubSub);
        let cfg_b = ChannelConfig::new("pb".to_string(), DeliveryMode::PubSub);
        let mut producer = wire.new_sender(&cfg_a).await.unwrap();
        let mut ca = wire.new_receiver(&cfg_a).await.unwrap();
        let mut cb = wire.new_receiver(&cfg_b).await.unwrap();
        producer.start().await.unwrap();
        ca.start().await.unwrap();
        cb.start().await.unwrap();

        producer
            .send_batch(&[make_msg(b"only-a")], None)
            .await
            .unwrap();

        let m = ca.receive().await.unwrap();
        assert_eq!(m.payload, b"only-a");

        assert!(wire.bus().get_pubsub_message("pb", 0).is_none());
    }
}
