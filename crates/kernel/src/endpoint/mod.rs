//! Endpoint abstractions: handlers, publishers, subscribers.
//!
//! - [`handler`] defines the `Handler<T>` trait that message consumers implement.
//! - `publisher` and `subscriber` modules are stubs (implemented in T5/T6).

pub mod handler;
pub mod publisher;
pub mod subscriber;
