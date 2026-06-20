//! Integration test for the work-queue (competing consumers) pattern.

use std::collections::BTreeMap;

use wirecrab_contrib_wire_memory::InMemoryWire;
use wirecrab_kernel::wire::{ChannelConfig, DeliveryMode, Lifecycle, Wire, WireMessage};

fn task(id: u8) -> WireMessage {
    WireMessage {
        headers: BTreeMap::new(),
        payload: vec![id],
        correlation_id: None,
        content_type: None,
        reply_to: None,
    }
}

#[tokio::test]
async fn work_queue_pattern() {
    let mut wire = InMemoryWire::new();
    wire.start().await.unwrap();

    let cfg = ChannelConfig::new("tasks.queue".to_string(), DeliveryMode::WorkQueue);
    let mut producer = wire.new_sender(&cfg).await.unwrap();
    let mut worker_a = wire.new_receiver(&cfg).await.unwrap();
    let mut worker_b = wire.new_receiver(&cfg).await.unwrap();

    producer.start().await.unwrap();
    worker_a.start().await.unwrap();
    worker_b.start().await.unwrap();

    // Publish 4 task messages up front: 1, 2, 3, 4.
    let batch: Vec<WireMessage> = (1..=4u8).map(task).collect();
    producer.send_batch(&batch, None).await.unwrap();

    // Spawn both workers concurrently; each drains 2 messages from the shared queue.
    let handle_a = tokio::spawn(async move {
        let mut got = Vec::new();
        for _ in 0..2 {
            got.push(worker_a.receive().await.unwrap().payload);
        }
        got
    });
    let handle_b = tokio::spawn(async move {
        let mut got = Vec::new();
        for _ in 0..2 {
            got.push(worker_b.receive().await.unwrap().payload);
        }
        got
    });

    let from_a = handle_a.await.unwrap();
    let from_b = handle_b.await.unwrap();

    // Combined total received must be exactly 4 (no losses, no duplicates).
    let mut all: Vec<Vec<u8>> = from_a.into_iter().chain(from_b).collect();
    assert_eq!(all.len(), 4, "expected exactly 4 delivered messages");

    // Sort to make the assertion order-independent: distribution between
    // workers is non-deterministic, but the combined set must be {1,2,3,4}.
    all.sort();
    assert_eq!(all, vec![vec![1], vec![2], vec![3], vec![4]]);
}
