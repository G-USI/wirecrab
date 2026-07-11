use crate::utils::structs::*;
use core::future::Future;
use core::pin::Pin;

/// Type-erased future for background tasks (consume loops, dispatchers).
///
/// Represents a `Future` that outputs `Result<(), AnyhowError>` and is
/// `Send + 'static`. Collected by [`Application`] and driven via
/// `try_join_all` inside [`Application::run`].
///
/// [`Application`]: crate::application::Application
/// [`Application::run`]: crate::application::Application::run
pub type BoxFuture = Pin<Box<dyn Future<Output = Result<(), AnyhowError>> + Send + 'static>>;
