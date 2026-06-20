//! Publisher endpoint.
//!
//! A [`Publisher`] encodes outgoing payloads via a bound [`Codec`], resolves
//! the channel address (substituting parameters from the payload when the
//! channel declares any), and dispatches a single [`WireMessage`] through a
//! [`Sender`].
//!
//! The codec is generic (`Publisher<C: Codec>`) rather than `dyn Codec`
//! because `Codec` is not object-safe (it has a generic method). A publisher
//! is constructed with one concrete codec and keeps it for its lifetime.

use crate::codec::{self, Codec};
use crate::document::channel::Channel;
use crate::utils::structs::*;
use crate::wire::{Sender, WireMessage};
use serde::ser::Serialize;

pub struct Publisher<C: Codec> {
    codec: C,
    channel: Channel,
    sender: Box<dyn Sender>,
}

impl<C: Codec> Publisher<C> {
    pub fn new(codec: C, channel: Channel, sender: Box<dyn Sender>) -> Self {
        Self {
            codec,
            channel,
            sender,
        }
    }

    /// Encode `payload`, resolve the channel address (if the channel declares
    /// any parameters), and dispatch it via [`Sender::send_batch`].
    ///
    /// When the channel has no parameters the address override passed to the
    /// sender is `None` (the sender is expected to use the static address it
    /// was configured with). When the channel declares parameters, they are
    /// extracted from the encoded bytes and substituted into
    /// [`Channel::address`], and the resolved address is passed as the
    /// override.
    pub async fn send<D>(&mut self, payload: &D) -> Result<(), AnyhowError>
    where
        D: Serialize + ?Sized,
    {
        let bytes = self.codec.encode(payload)?;

        let address_override = if self.channel.parameters.is_empty() {
            None
        } else {
            let params = codec::extract_parameters(&self.codec, &bytes, &self.channel.parameters)?;
            let base = self.channel.address.as_deref().unwrap_or("");
            let resolved = codec::resolve_address(base, &params)?;
            Some(resolved)
        };

        let msg = WireMessage {
            payload: bytes,
            content_type: Some(String::from("application/json")),
            headers: BTreeMap::new(),
            correlation_id: None,
            reply_to: None,
        };

        self.sender
            .send_batch(&[msg], address_override.as_deref())
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::channel::AddressParameter;
    use crate::wire::Lifecycle;
    use async_trait::async_trait;
    use serde::de::DeserializeOwned;
    use std::sync::{Arc, Mutex};

    fn s(lit: &str) -> String {
        String::from(lit)
    }

    /// Mock codec mirroring the `TestCodec` pattern from `codec.rs` tests:
    /// `extract_field` looks up the trailing `/key` of the location in a
    /// hardcoded table, ignoring the actual bytes.
    struct TestCodec {
        values: BTreeMap<String, String>,
    }

    impl Codec for TestCodec {
        fn encode<D>(&self, _value: &D) -> Result<Vec<u8>, AnyhowError>
        where
            D: Serialize + ?Sized,
        {
            Ok(Vec::new())
        }

        fn decode<D>(&self, _bytes: &[u8]) -> Result<D, AnyhowError>
        where
            D: DeserializeOwned,
        {
            Err(anyhow::anyhow!("TestCodec::decode not implemented"))
        }

        fn extract_field(&self, _bytes: &[u8], location: &str) -> Result<String, AnyhowError> {
            let key = location
                .rsplit('/')
                .next()
                .ok_or_else(|| anyhow::anyhow!("invalid location: {location}"))?;
            self.values
                .get(key)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("no value for: {key}"))
        }
    }

    type SentBatch = (Vec<WireMessage>, Option<String>);

    /// Captures every `send_batch` call: the messages slice (cloned) and the
    /// optional address override.
    struct MockSender {
        sent: Arc<Mutex<Vec<SentBatch>>>,
    }

    #[async_trait]
    impl Lifecycle for MockSender {
        async fn start(&mut self) -> Result<(), AnyhowError> {
            Ok(())
        }
        async fn stop(&mut self) -> Result<(), AnyhowError> {
            Ok(())
        }
    }

    #[async_trait]
    impl Sender for MockSender {
        async fn send_batch(
            &mut self,
            messages: &[WireMessage],
            address_override: Option<&str>,
        ) -> Result<(), AnyhowError> {
            self.sent
                .lock()
                .unwrap()
                .push((messages.to_vec(), address_override.map(String::from)));
            Ok(())
        }
    }

    fn make_channel(address: Option<&str>, params: &[(&str, &str)]) -> Channel {
        let mut parameters = BTreeMap::new();
        for (name, loc) in params {
            parameters.insert(
                s(name),
                AddressParameter {
                    description: None,
                    location: s(loc),
                    key: s(name),
                },
            );
        }
        Channel {
            address: address.map(s),
            title: None,
            summary: None,
            description: None,
            messages: BTreeMap::new(),
            parameters,
            tags: Vec::new(),
            external_docs: None,
            key: s("test-channel"),
        }
    }

    #[tokio::test]
    async fn publish_static_address_no_override() {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let sender = MockSender { sent: sent.clone() };
        let codec = TestCodec {
            values: BTreeMap::new(),
        };
        let channel = make_channel(Some("events/static"), &[]);

        let mut pub_ = Publisher::new(codec, channel, Box::new(sender));

        // Any Serialize value works — TestCodec::encode ignores it.
        pub_.send(&42i32).await.expect("send ok");

        let captured = sent.lock().unwrap();
        assert_eq!(captured.len(), 1, "exactly one send_batch call");
        let (msgs, addr) = &captured[0];
        assert!(addr.is_none(), "no parameters → no address_override");
        assert_eq!(msgs.len(), 1, "exactly one message in the batch");
        assert_eq!(
            msgs[0].content_type.as_deref(),
            Some("application/json"),
            "content_type hardcoded to application/json",
        );
        assert!(msgs[0].headers.is_empty());
        assert!(msgs[0].correlation_id.is_none());
        assert!(msgs[0].reply_to.is_none());
    }

    #[tokio::test]
    async fn publish_parameterized_address_resolved() {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let sender = MockSender { sent: sent.clone() };
        let mut values = BTreeMap::new();
        values.insert(s("user_id"), s("42"));
        let codec = TestCodec { values };
        let channel = make_channel(
            Some("users/{user_id}"),
            &[("user_id", "$message.payload#/user_id")],
        );

        let mut pub_ = Publisher::new(codec, channel, Box::new(sender));

        pub_.send(&"anything").await.expect("send ok");

        let captured = sent.lock().unwrap();
        assert_eq!(captured.len(), 1);
        let (msgs, addr) = &captured[0];
        assert_eq!(
            addr.as_deref(),
            Some("users/42"),
            "parameterized address must be substituted from payload",
        );
        assert_eq!(msgs.len(), 1);
    }

    #[tokio::test]
    async fn publish_multiple_calls_accumulate() {
        // Sanity check: the same publisher can be reused; each send is a
        // distinct batch.
        let sent = Arc::new(Mutex::new(Vec::new()));
        let sender = MockSender { sent: sent.clone() };
        let codec = TestCodec {
            values: BTreeMap::new(),
        };
        let channel = make_channel(Some("events/static"), &[]);

        let mut pub_ = Publisher::new(codec, channel, Box::new(sender));

        pub_.send(&1i32).await.unwrap();
        pub_.send(&2i32).await.unwrap();
        pub_.send(&3i32).await.unwrap();

        let captured = sent.lock().unwrap();
        assert_eq!(captured.len(), 3, "three sends → three batches");
        for (_msgs, addr) in captured.iter() {
            assert!(addr.is_none());
        }
    }
}
