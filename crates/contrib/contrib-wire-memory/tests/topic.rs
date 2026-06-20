//! Topic (parameterized addressing) pattern using `address_override`.
//!
//! Demonstration that a producer configured for a base address can route to
//! concrete parameterized addresses per-batch. No wildcard matching — the
//! bus routes purely on the overridden address.

use std::collections::BTreeMap;

use wirecrab_contrib_wire_memory::InMemoryWire;
use wirecrab_kernel::wire::{ChannelConfig, DeliveryMode, Lifecycle, Wire, WireMessage};

fn json_msg(payload: &[u8]) -> WireMessage {
    WireMessage {
        headers: BTreeMap::new(),
        payload: payload.to_vec(),
        correlation_id: None,
        content_type: Some("application/json".to_string()),
        reply_to: None,
    }
}

#[tokio::test]
async fn topic_parameterized_addressing() {
    let mut wire = InMemoryWire::new();
    wire.start().await.unwrap();

    let base_cfg = ChannelConfig::new("weather".to_string(), DeliveryMode::WorkQueue);
    let mut producer = wire.new_sender(&base_cfg).await.unwrap();

    let nyc_cfg = ChannelConfig::new("weather.NYC.high".to_string(), DeliveryMode::WorkQueue);
    let mut consumer_nyc = wire.new_receiver(&nyc_cfg).await.unwrap();

    let la_cfg = ChannelConfig::new("weather.LA.low".to_string(), DeliveryMode::WorkQueue);
    let mut consumer_la = wire.new_receiver(&la_cfg).await.unwrap();

    producer.start().await.unwrap();
    consumer_nyc.start().await.unwrap();
    consumer_la.start().await.unwrap();

    let nyc_msg = json_msg(br#"{"city":"NYC","level":"high"}"#);
    producer
        .send_batch(&[nyc_msg], Some("weather.NYC.high"))
        .await
        .unwrap();

    let la_msg = json_msg(br#"{"city":"LA","level":"low"}"#);
    producer
        .send_batch(&[la_msg], Some("weather.LA.low"))
        .await
        .unwrap();

    let received_nyc = consumer_nyc.receive().await.unwrap();
    let received_la = consumer_la.receive().await.unwrap();

    assert_eq!(received_nyc.payload, br#"{"city":"NYC","level":"high"}"#);
    assert_eq!(received_la.payload, br#"{"city":"LA","level":"low"}"#);

    assert_ne!(
        received_nyc.payload, received_la.payload,
        "consumers must receive distinct messages"
    );

    assert!(
        wire.bus().get_work_message("weather").is_none(),
        "base address must be empty when every send used an override"
    );
}
