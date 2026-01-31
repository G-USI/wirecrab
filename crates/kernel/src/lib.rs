#![no_std]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

pub mod prelude {
    pub use crate::utils::structs::*;
}

pub mod document;
pub mod utils;

pub mod application {}
pub mod codec {}
pub mod endpoint {}
pub mod error;
pub mod wire;
