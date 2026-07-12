use wirecrab_contrib_codec_json::JsonCodec;
use wirecrab_contrib_wire_memory::InMemoryWire;
use wirecrab_kernel::application::Application;
use wirecrab_kernel::document::Schema;
use wirecrab_kernel::endpoint::handler::{HandlerError, MessageContext};
use wirecrab_kernel::wire::{ChannelConfig, DeliveryMode, Lifecycle, Wire};
use wirecrab_macros::asyncapi;

#[asyncapi("spec/ping.asyncapi.yaml")]
struct PingApp;

#[tokio::main]
async fn main() {
    let mut wire = InMemoryWire::new();
    wire.start().await.unwrap();

    let ping_schema = Schema {
        format: String::from("application/json"),
        source: String::from(
            r#"{"type":"object","properties":{"event":{"type":"string","const":"ping"},"count":{"type":"integer"}},"required":["event"]}"#,
        ),
    };

    // Subscriber
    let sub_codec = JsonCodec::new(&ping_schema).unwrap();
    let cfg = ChannelConfig::new(String::from("/ping/pubsub"), DeliveryMode::WorkQueue);
    let receiver = wire.new_receiver(&cfg).await.unwrap();
    let handler = |msg: Ping, _ctx: &MessageContext| async move {
        println!("Received: {msg:?}");
        Ok::<(), HandlerError>(())
    };
    let subscriber = PingApp::subscriber_receive_ping(sub_codec, handler, receiver);

    // Publisher
    let pub_codec = JsonCodec::new(&ping_schema).unwrap();
    let mut sender = wire.new_sender(&cfg).await.unwrap();
    sender.start().await.unwrap();
    let mut publisher = PingApp::publisher_send_ping(pub_codec, sender);

    // Register + send + run
    let mut app = Application::new();
    app.register_subscriber(subscriber).await.unwrap();

    let ping = Ping {
        event: String::from("ping"),
        count: Some(42),
    };
    publisher.send(&ping).await.unwrap();

    tokio::time::timeout(std::time::Duration::from_millis(500), app.run())
        .await
        .ok();
}
