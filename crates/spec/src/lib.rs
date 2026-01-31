//
// SPDX-License-Identifier: Apache-2.0

//! AsyncAPI spec parser and validator.
//!
//! This crate provides parsing and validation of AsyncAPI 3.0/3.1 specifications.
//! It operates on [`kernel::document`] types from wirecrab-kernel crate.

pub mod ast;

pub use kernel::document;
