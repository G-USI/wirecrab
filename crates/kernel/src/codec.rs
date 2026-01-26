#![forbid(unsafe_code)]

use crate::error::Result;
use crate::models::Message;
use async_trait::async_trait;
use std::any::Any;

#[async_trait]
pub trait Codec: Send + Sync {
    async fn encode_any(&self, value: &dyn Any) -> Result<Vec<u8>>;

    async fn decode_any(&self, data: &[u8]) -> Result<Box<dyn Any>>;

    fn as_any(&self) -> &dyn Any;
}

pub trait CodecFactory: Send + Sync {
    fn create_codec(&self, message: &Message) -> Result<Box<dyn Codec>>;
}
