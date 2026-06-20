use std::collections::BTreeMap;

use serde_json::json;
use wirecrab_contrib_codec_json::JsonCodec;
use wirecrab_contrib_wire_memory::InMemoryWire;
use wirecrab_kernel::codec::Codec;
use wirecrab_kernel::document::Schema;
use wirecrab_kernel::wire::{ChannelConfig, DeliveryMode, Lifecycle, Wire, WireMessage};

#[tokio::test]
async fn pub_sub_pattern() {
    let mut wire = InMemoryWire::new();
    wire.start().await.expect("wire should start");

    let cfg = ChannelConfig::new("ping.channel".to_string(), DeliveryMode::WorkQueue);

    let mut producer = wire
        .new_sender(&cfg)
        .await
        .expect("sender should be created");
    let mut consumer = wire
        .new_receiver(&cfg)
        .await
        .expect("receiver should be created");
    producer.start().await.expect("producer should start");
    consumer.start().await.expect("consumer should start");

    let codec = JsonCodec::new(&Schema {
        format: "application/json".to_string(),
        source: r#"{"type":"object","properties":{"event":{"type":"string","const":"ping"}}}"#
            .to_string(),
    })
    .expect("Ping schema should compile");

    for _ in 0..3 {
        let bytes = codec
            .encode(&json!({"event": "ping"}))
            .expect("valid Ping should encode");
        let msg = WireMessage {
            headers: BTreeMap::new(),
            payload: bytes,
            correlation_id: None,
            content_type: Some("application/json".to_string()),
            reply_to: None,
        };
        producer
            .send_batch(&[msg], None)
            .await
            .expect("send_batch should succeed");
    }

    for _ in 0..3 {
        let mut incoming = consumer
            .receive()
            .await
            .expect("consumer should receive a message");

        assert_eq!(
            incoming.content_type,
            Some("application/json".to_string()),
            "content_type header should round-trip",
        );

        let value: serde_json::Value = codec
            .decode(&incoming.payload)
            .expect("received bytes should decode as Ping");
        assert_eq!(
            value,
            json!({"event": "ping"}),
            "decoded payload must match the original Ping message",
        );

        assert!(
            incoming.is_pending(),
            "message must be Pending before ack",
        );
        incoming.ack();
        assert!(
            incoming.is_acknowledged(),
            "message must be Acked after ack()",
        );
    }
}
