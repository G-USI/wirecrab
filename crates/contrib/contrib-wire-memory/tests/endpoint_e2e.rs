//! End-to-end integration test: Publisher + Subscriber + Application +
//! InMemoryWire + JsonCodec.
//!
//! Proves the full wirecrab stack works together:
//! - [`Publisher`] encodes payloads via [`JsonCodec`] and dispatches through an
//!   [`InMemoryWire`] sender.
//! - [`InMemoryWire`] routes PubSub messages to registered consumer queues.
//! - [`Subscriber`]'s consume loop decodes each message and dispatches to a
//!   [`Handler`] closure.
//! - [`Application`] collects the Subscriber's loop into a [`Runner`].
//! - [`Runner`] drives the loop inside a `tokio` task spawned by the test.
//!
//! # Error isolation (W13 fix)
//!
//! The second test proves that a transient [`HandlerError`] does NOT break the
//! consume loop — subsequent messages are still processed. This is the W13 fix:
//! the only way the loop exits normally is when the wire reports an error on
//! [`Receiver::receive`](wirecrab_kernel::wire::Receiver::receive).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;
use wirecrab_contrib_codec_json::JsonCodec;
use wirecrab_contrib_wire_memory::InMemoryWire;
use wirecrab_kernel::application::Application;
use wirecrab_kernel::document::Schema;
use wirecrab_kernel::document::channel::Channel;
use wirecrab_kernel::document::common::Item;
use wirecrab_kernel::endpoint::handler::{HandlerError, MessageContext};
use wirecrab_kernel::endpoint::publisher::Publisher;
use wirecrab_kernel::endpoint::subscriber::Subscriber;
use wirecrab_kernel::wire::{ChannelConfig, DeliveryMode, Lifecycle, Wire};

const EVENT_SCHEMA: &str = r#"{"type":"object","properties":{"event":{"type":"string"}}}"#;

fn event_schema() -> Schema {
    Schema {
        format: "application/json".to_string(),
        source: EVENT_SCHEMA.to_string(),
    }
}

fn static_channel(address: &str) -> Channel {
    Channel {
        address: Some(address.to_string()),
        title: None,
        summary: None,
        description: None,
        messages: Vec::new(),
        parameters: Vec::new(),
        tags: Vec::new(),
        external_docs: None,
    }
}

/// Yield to the tokio executor until `counter` reaches `target`, or we exhaust
/// `max_yields` attempts (at which point the caller's assertion will fail with
/// a clear count mismatch).
///
/// The spawned Runner task needs CPU time to poll the Subscriber's consume
/// loop to completion. The workspace tokio does not enable the `time` feature,
/// so we cannot use `tokio::time::sleep`; cooperative yielding is sufficient
/// because the InMemoryWire is intra-process.
async fn wait_for(counter: &Arc<AtomicUsize>, target: usize, max_yields: usize) {
    for _ in 0..max_yields {
        if counter.load(Ordering::SeqCst) >= target {
            return;
        }
        tokio::task::yield_now().await;
    }
}

/// Happy path: Publisher sends 3 messages → Subscriber handler called 3 times.
#[tokio::test]
async fn endpoint_e2e_happy_path() {
    let mut wire = InMemoryWire::new();
    wire.start().await.expect("wire should start");

    let address = "test/e2e/happy";
    let cfg = ChannelConfig::new(address.to_string(), DeliveryMode::PubSub);

    // Sender must be started before being moved into the Publisher (Publisher
    // does not auto-start its sender).
    let mut sender = wire.new_sender(&cfg).await.expect("sender created");
    sender.start().await.expect("sender started");
    let receiver = wire.new_receiver(&cfg).await.expect("receiver created");

    // Two codec instances with the same schema (JsonCodec does not impl Clone).
    let pub_codec = JsonCodec::new(&event_schema()).expect("pub codec compiles");
    let sub_codec = JsonCodec::new(&event_schema()).expect("sub codec compiles");
    let channel = static_channel(address);

    let processed = Arc::new(AtomicUsize::new(0));
    let processed_handler = processed.clone();
    let handler = move |_payload: serde_json::Value, _ctx: &MessageContext| {
        let p = processed_handler.clone();
        async move {
            p.fetch_add(1, Ordering::SeqCst);
            Ok::<(), HandlerError>(())
        }
    };

    let subscriber = Subscriber::new(sub_codec, handler, receiver);
    let mut publisher = Publisher::new(pub_codec, channel, sender);

    let mut app = Application::new();
    app.register_subscriber(subscriber)
        .await
        .expect("register_subscriber ok");
    assert_eq!(app.len(), 1, "one subscriber future must be registered");

    let runner = app.into_runner();
    // Spawn the runner drive in a tokio task — the no-spawn pattern: the
    // library returns the Runner, the caller provides the executor.
    let _runner_handle = tokio::spawn(async move { runner.run().await });

    let payload = json!({"event": "ping"});
    publisher
        .send(&payload)
        .await
        .expect("send 1 should succeed");
    publisher
        .send(&payload)
        .await
        .expect("send 2 should succeed");
    publisher
        .send(&payload)
        .await
        .expect("send 3 should succeed");

    wait_for(&processed, 3, 256).await;

    assert_eq!(
        processed.load(Ordering::SeqCst),
        3,
        "happy path: handler must be called once per published message"
    );

    // Best-effort cleanup; the spawned task is aborted when the test's tokio
    // runtime is torn down at function return.
    wire.stop().await.expect("wire stopped cleanly");
}

/// W13 fix proof: handler returns Transient error on the 2nd call, but the 3rd
/// message must still be processed.
#[tokio::test]
async fn endpoint_e2e_error_isolation() {
    let mut wire = InMemoryWire::new();
    wire.start().await.expect("wire should start");

    let address = "test/e2e/iso";
    let cfg = ChannelConfig::new(address.to_string(), DeliveryMode::PubSub);

    let mut sender = wire.new_sender(&cfg).await.expect("sender created");
    sender.start().await.expect("sender started");
    let receiver = wire.new_receiver(&cfg).await.expect("receiver created");

    let pub_codec = JsonCodec::new(&event_schema()).expect("pub codec compiles");
    let sub_codec = JsonCodec::new(&event_schema()).expect("sub codec compiles");
    let channel = static_channel(address);

    let calls = Arc::new(AtomicUsize::new(0));
    let calls_handler = calls.clone();
    let handler = move |_payload: serde_json::Value, _ctx: &MessageContext| {
        let c = calls_handler.clone();
        async move {
            let n = c.fetch_add(1, Ordering::SeqCst);
            if n == 1 {
                return Err::<(), HandlerError>(HandlerError::Transient(anyhow::anyhow!(
                    "transient blip on msg 2"
                )));
            }
            Ok(())
        }
    };

    let subscriber = Subscriber::new(sub_codec, handler, receiver);
    let mut publisher = Publisher::new(pub_codec, channel, sender);

    let mut app = Application::new();
    app.register_subscriber(subscriber)
        .await
        .expect("register_subscriber ok");

    let runner = app.into_runner();
    let _runner_handle = tokio::spawn(async move { runner.run().await });

    let payload = json!({"event": "ping"});
    publisher
        .send(&payload)
        .await
        .expect("send 1 should succeed");
    publisher
        .send(&payload)
        .await
        .expect("send 2 should succeed");
    publisher
        .send(&payload)
        .await
        .expect("send 3 should succeed");

    wait_for(&calls, 3, 256).await;

    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "W13 fix: Transient error on msg 2 must NOT stop the loop — \
         msg 3 must still be processed (got {} calls)",
        calls.load(Ordering::SeqCst)
    );

    wire.stop().await.expect("wire stopped cleanly");
}

/// Parameterized address: Publisher with `users/{user_id}` template extracts
/// user_id from payload, resolves to `users/42`, sends with address_override.
/// Subscriber listens on the resolved address and receives the message.
#[tokio::test]
async fn endpoint_e2e_parameterized_address() {
    let mut wire = InMemoryWire::new();
    wire.start().await.expect("wire should start");

    let resolved_address = "users/42";
    let sub_cfg = ChannelConfig::new(resolved_address.to_string(), DeliveryMode::PubSub);
    let receiver = wire.new_receiver(&sub_cfg).await.expect("receiver created");

    let schema_source =
        r#"{"type":"object","properties":{"user_id":{"type":"string"},"event":{"type":"string"}}}"#;
    let pub_schema = Schema {
        format: "application/json".to_string(),
        source: schema_source.to_string(),
    };
    let sub_schema = Schema {
        format: "application/json".to_string(),
        source: schema_source.to_string(),
    };

    let pub_codec = JsonCodec::new(&pub_schema).expect("pub codec compiles");
    let sub_codec = JsonCodec::new(&sub_schema).expect("sub codec compiles");

    let params = vec![Item {
        key: "user_id".to_string(),
        item: wirecrab_kernel::document::channel::AddressParameter {
            description: None,
            location: "$message.payload#/user_id".to_string(),
        },
    }];
    let pub_channel = Channel {
        address: Some("users/{user_id}".to_string()),
        title: None,
        summary: None,
        description: None,
        messages: Vec::new(),
        parameters: params,
        tags: Vec::new(),
        external_docs: None,
    };

    let processed = Arc::new(AtomicUsize::new(0));
    let processed_handler = processed.clone();
    let handler = move |_payload: serde_json::Value, _ctx: &MessageContext| {
        let p = processed_handler.clone();
        async move {
            p.fetch_add(1, Ordering::SeqCst);
            Ok::<(), HandlerError>(())
        }
    };

    let subscriber = Subscriber::new(sub_codec, handler, receiver);

    let mut sender = wire
        .new_sender(&ChannelConfig::new(
            "users/{user_id}".to_string(),
            DeliveryMode::PubSub,
        ))
        .await
        .expect("sender created");
    sender.start().await.expect("sender started");

    let mut publisher = Publisher::new(pub_codec, pub_channel, sender);

    let mut app = Application::new();
    app.register_subscriber(subscriber)
        .await
        .expect("register_subscriber ok");

    let runner = app.into_runner();
    let _runner_handle = tokio::spawn(async move { runner.run().await });

    let payload = json!({"user_id": "42", "event": "ping"});
    publisher
        .send(&payload)
        .await
        .expect("send should succeed with resolved address");

    wait_for(&processed, 1, 256).await;

    assert_eq!(
        processed.load(Ordering::SeqCst),
        1,
        "parameterized address: subscriber on 'users/42' must receive message \
         published via template 'users/{{user_id}}' with payload user_id=42"
    );

    wire.stop().await.expect("wire stopped cleanly");
}
