pub mod structs {
    #[cfg(feature = "std")]
    pub use std::vec::Vec;

    #[cfg(not(feature = "std"))]
    pub use alloc::vec::Vec;

    #[cfg(feature = "std")]
    pub use std::boxed::Box;

    #[cfg(not(feature = "std"))]
    pub use alloc::boxed::Box;

    #[cfg(feature = "std")]
    pub use std::string::String;

    #[cfg(not(feature = "std"))]
    pub use alloc::string::String;

    #[cfg(feature = "std")]
    pub use std::sync::Arc;

    #[cfg(not(feature = "std"))]
    pub use alloc::sync::Arc;

    #[cfg(feature = "std")]
    pub use std::collections::VecDeque;

    #[cfg(not(feature = "std"))]
    pub use alloc::collections::VecDeque;

    #[cfg(feature = "std")]
    pub use std::collections::BTreeMap;

    #[cfg(not(feature = "std"))]
    pub use alloc::collections::BTreeMap;

    pub use anyhow::Error as AnyhowError;
    pub use async_trait::async_trait;
    pub use thiserror::Error as ThisError;

    #[cfg(feature = "std")]
    pub use std::sync::Arc as Shared;

    #[cfg(not(feature = "std"))]
    pub use alloc::sync::Arc as Shared;

    #[cfg(feature = "no-atomics")]
    pub trait ThreadSafe {}

    #[cfg(not(feature = "no-atomics"))]
    pub trait ThreadSafe: Send + Sync {}

    #[cfg(feature = "no-atomics")]
    impl<T> ThreadSafe for T {}

    #[cfg(not(feature = "no-atomics"))]
    impl<T: Send + Sync> ThreadSafe for T {}
}
