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
    pub use std::rc::Rc;

    #[cfg(not(feature = "std"))]
    pub use alloc::rc::Rc;

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

    #[cfg(feature = "std")]
    pub use std::collections::BTreeSet;

    #[cfg(not(feature = "std"))]
    pub use alloc::collections::BTreeSet;

    #[cfg(feature = "std")]
    pub use std::collections::LinkedList;

    #[cfg(not(feature = "std"))]
    pub use alloc::collections::LinkedList;

    #[cfg(feature = "std")]
    pub use std::collections::BinaryHeap;

    #[cfg(not(feature = "std"))]
    pub use alloc::collections::BinaryHeap;

    pub use core::option::Option;
    pub use core::result::Result;

    pub use anyhow::Error as AnyhowError;
    pub use async_trait::async_trait;
    pub use thiserror::Error as ThisError;
}
