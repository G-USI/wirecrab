//! Chat example: demonstrates typify generating richer types.
//!
//! - Multiple message types per app
//! - Enums (role, reason)
//! - Arrays (mentions)
//! - Nested objects (metadata: map<string, anything>)
//! - UUID and date-time formats
//! - Newtype wrappers for constrained strings
//! - Schemas accessed via generated `schema_*` accessors
//!
//! Run with: `cargo run -p wirecrab-example-chat`

use wirecrab_contrib_codec_json::JsonCodec;
use wirecrab_contrib_wire_memory::InMemoryWire;
use wirecrab_kernel::application::Application;
use wirecrab_kernel::endpoint::handler::{HandlerError, MessageContext};
use wirecrab_kernel::wire::{ChannelConfig, DeliveryMode, Lifecycle, Wire};
use wirecrab_macros::asyncapi;

#[asyncapi("spec/chat.asyncapi.yaml")]
struct ChatApp;

#[tokio::main]
async fn main() {
    let mut wire = InMemoryWire::new();
    wire.start().await.unwrap();

    // ---- Channel: /chat/messages (Pub/Sub) ----
    let msg_cfg = ChannelConfig::new(String::from("/chat/messages"), DeliveryMode::PubSub);

    let pub_codec = JsonCodec::new(&ChatApp::schema_chatmessage()).unwrap();
    let sub_codec = JsonCodec::new(&ChatApp::schema_chatmessage()).unwrap();

    let mut sender = wire.new_sender(&msg_cfg).await.unwrap();
    sender.start().await.unwrap();
    let receiver = wire.new_receiver(&msg_cfg).await.unwrap();

    let handler = |msg: ChatMessage, _ctx: &MessageContext| async move {
        println!("[chat] {msg:?}");
        Ok::<(), HandlerError>(())
    };
    let subscriber = ChatApp::subscriber_consume_chat_message(sub_codec, handler, receiver);
    let mut publisher = ChatApp::publisher_publish_chat_message(pub_codec, sender);

    // ---- Channel: /chat/events (WorkQueue) ----
    let evt_cfg = ChannelConfig::new(String::from("/chat/events"), DeliveryMode::WorkQueue);

    let evt_pub_codec = JsonCodec::new(&ChatApp::schema_userjoined()).unwrap();
    let evt_sub_codec = JsonCodec::new(&ChatApp::schema_userjoined()).unwrap();

    let mut evt_sender = wire.new_sender(&evt_cfg).await.unwrap();
    evt_sender.start().await.unwrap();
    let evt_receiver = wire.new_receiver(&evt_cfg).await.unwrap();

    let evt_handler = |msg: UserJoined, _ctx: &MessageContext| async move {
        println!("[event] user joined: {msg:?}");
        Ok::<(), HandlerError>(())
    };
    let evt_subscriber =
        ChatApp::subscriber_consume_user_event(evt_sub_codec, evt_handler, evt_receiver);
    let mut evt_publisher = ChatApp::publisher_publish_user_event(evt_pub_codec, evt_sender);

    // ---- Register subscribers ----
    let mut app = Application::new();
    app.register_subscriber(subscriber).await.unwrap();
    app.register_subscriber(evt_subscriber).await.unwrap();

    // ---- Publish events ----
    let alice_id = uuid::Uuid::new_v4();
    let bob_id = uuid::Uuid::new_v4();

    let msg = ChatMessage {
        from: alice_id,
        to: Some(bob_id),
        text: String::from("hello world"),
        mentions: vec![String::from("@admin"), String::from("@team")],
        metadata: serde_json::Map::new(),
    };
    publisher.send(&msg).await.unwrap();

    let joined = UserJoined {
        user_id: alice_id,
        username: UserJoinedUsername(String::from("alice")),
        role: UserJoinedRole::Member,
        joined_at: chrono::Utc::now(),
    };
    evt_publisher.send(&joined).await.unwrap();

    tokio::time::timeout(std::time::Duration::from_millis(500), app.run())
        .await
        .ok();
}
