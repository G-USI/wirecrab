//! Endpoint abstractions: handlers, publishers, subscribers.
//!
//! - [`handler`] defines the `Handler<T>` trait that message consumers implement.
//! - [`publisher`] defines the `Publisher<C>` endpoint for sending messages.
//! - [`subscriber`] defines the `Subscriber<C, H>` endpoint for consume loops.

pub mod handler;
pub mod publisher;
pub mod subscriber;
