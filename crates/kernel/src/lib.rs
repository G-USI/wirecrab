#![forbid(unsafe_code)]

pub mod application;
pub mod codec;
pub mod endpoint;
pub mod error;
pub mod models;
pub mod wire;

pub use application::{Application, ApplicationBuilder, ApplicationRuntime};
pub use codec::{Codec, CodecFactory};
pub use endpoint::{Consumer, Producer, RpcClient, RpcHandler, RpcServer};
pub use error::{ApplicationError, CodecError, ConnectionError, EndpointError, Error, Result};
pub use models::operation::{Action, Reply};
pub use models::{Channel, Message, Operation, OperationBindings, ParameterValue};
pub use wire::{EndpointParams, Wire, WireFactory};
