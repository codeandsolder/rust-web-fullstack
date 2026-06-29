//! Concrete [`ServiceModule`](crate::module::ServiceModule) implementations.
//!
//! Each sub-module provides a mock service that can be mounted under the
//! gateway's path tree:
//!
//! * [`monitor`] — status dashboard
//! * [`proxy`] — IP proxy / VPN check
//! * [`search`] — full-text search simulation

pub mod monitor;
pub mod proxy;
pub mod search;
