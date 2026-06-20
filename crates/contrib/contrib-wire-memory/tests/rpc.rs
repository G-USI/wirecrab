//! Integration test: RPC (request-reply) pattern with the in-memory wire and JSON codec.
//!
//! Verifies the end-to-end RPC cycle:
//! 1. Client encodes a Ping request and sends it to `"rpc.requests"` with a
//!    `correlation_id` and `reply_to` address.
//! 2. Server (spawned tokio task) receives the request, extracts the
//!    `correlation_id`, encodes a Pong response, and sends it to
//!    `"rpc.replies"` with the same `correlation_id`.
//! 3. Client receives the reply, verifies the `correlation_id` matches, and
//!    decodes the Pong payload.

use std::collections::BTreeMap;

use serde_json::json;
use wirecrab_contrib_codec_json::JsonCodec;
use wirecrab_contrib_wire_memory::InMemoryWire;
use wirecrab_kernel::codec::Codec;
use wirecrab_kernel::document::Schema;
use wirecrab_kernel::wire::{ChannelConfig, DeliveryMode, Lifecycle, Wire, WireMessage};

const PING_SCHEMA: &str =
    r#"{"type":"object","properties":{"event":{"type":"string","const":"ping"}}}"#;
const PONG_SCHEMA: &str =
    r#"{"type":"object","properties":{"event":{"type":"string","const":"pong"}}}"#;

#[tokio::test]
async fn rpc_request_reply_cycle() {
    // --- Wire setup ---------------------------------------------------------
    let mut wire = InMemoryWire::new();
    wire.start().await.expect("wire should start");

    // --- Channel configs ----------------------------------------------------
    let req_cfg = ChannelConfig::new("rpc.requests".to_string(), DeliveryMode::WorkQueue);
    let reply_cfg = ChannelConfig::new("rpc.replies".to_string(), DeliveryMode::WorkQueue);

    // --- Endpoints: producer + consumer for requests and replies ------------
    let mut request_producer = wire
        .new_sender(&req_cfg)
        .await
        .expect("request sender should be created");
    let mut request_consumer = wire
        .new_receiver(&req_cfg)
        .await
        .expect("request receiver should be created");
    let mut reply_producer = wire
        .new_sender(&reply_cfg)
        .await
        .expect("reply sender should be created");
    let mut reply_consumer = wire
        .new_receiver(&reply_cfg)
        .await
        .expect("reply receiver should be created");

    request_producer
        .start()
        .await
        .expect("request producer should start");
    request_consumer
        .start()
        .await
        .expect("request consumer should start");
    reply_producer
        .start()
        .await
        .expect("reply producer should start");
    reply_consumer
        .start()
        .await
        .expect("reply consumer should start");

    // --- Client-side Pong codec (server builds its own inside the task) -----
    let pong_codec = JsonCodec::new(&Schema {
        format: "application/json".to_string(),
        source: PONG_SCHEMA.to_string(),
    })
    .expect("Pong schema should compile");

    // --- Server task: receive Ping, reply with Pong + same correlation_id ---
    let server_handle = tokio::spawn(async move {
        let req = request_consumer
            .receive()
            .await
            .expect("server should receive a request");

        let cid = req.correlation_id.clone();

        let server_pong_codec = JsonCodec::new(&Schema {
            format: "application/json".to_string(),
            source: PONG_SCHEMA.to_string(),
        })
        .expect("Pong schema should compile inside server task");
        let pong_bytes = server_pong_codec
            .encode(&json!({"event": "pong"}))
            .expect("valid Pong should encode");

        let reply = WireMessage {
            headers: BTreeMap::new(),
            payload: pong_bytes,
            correlation_id: cid,
            content_type: Some("application/json".to_string()),
            reply_to: None,
        };
        reply_producer
            .send_batch(&[reply], None)
            .await
            .expect("server should send reply");
    });

    // --- Client: encode Ping, send with correlation_id + reply_to -----------
    let ping_codec = JsonCodec::new(&Schema {
        format: "application/json".to_string(),
        source: PING_SCHEMA.to_string(),
    })
    .expect("Ping schema should compile");

    let ping_bytes = ping_codec
        .encode(&json!({"event": "ping"}))
        .expect("valid Ping should encode");

    let request = WireMessage {
        headers: BTreeMap::new(),
        payload: ping_bytes,
        correlation_id: Some("c1".to_string()),
        content_type: Some("application/json".to_string()),
        reply_to: Some("rpc.replies".to_string()),
    };
    request_producer
        .send_batch(&[request], None)
        .await
        .expect("client should send request");

    // --- Client: receive reply, verify correlation_id + Pong payload --------
    let reply = reply_consumer
        .receive()
        .await
        .expect("client should receive a reply");

    server_handle
        .await
        .expect("server task should complete successfully");

    assert_eq!(
        reply.correlation_id.as_deref(),
        Some("c1"),
        "correlation_id must match the original request",
    );

    let pong_value: serde_json::Value = pong_codec
        .decode(&reply.payload)
        .expect("reply payload should decode as Pong");
    assert_eq!(
        pong_value,
        json!({"event": "pong"}),
        "decoded reply must be a Pong message",
    );
}
