//! This library provides access to [`GCE metadata service`][`metadata`].
//!
//! # Example
//! ```rust
//! use gcemeta::*;
//!
//! println!("on_gce = {:?}", on_gce());
//! println!("project_id = {:?}", project_id());
//! ```
//!
//! [`metadata`]: https://developers.google.com/compute/docs/metadata

mod error;
mod metadata;

pub use error::{Error, ErrorKind, Result};
pub use metadata::*;
