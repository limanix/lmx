//! # lmx-ipc
//!
//! Calls between `lmx` and the guest owner daemon `lmxd`.
//!
//! `lmxd` serves gRPC on the Unix socket [`SOCKET_PATH`]. This crate holds what both sides share:
//!
//! | Item          | Is                                                                    |
//! |---------------|-----------------------------------------------------------------------|
//! | [`proto`]     | the `lmx.v1.Owner` service, generated from `proto/lmx/v1/owner.proto` |
//! | [`connect`]   | a client of that service over the socket                              |
//! | `From` impls  | conversions between its messages and the `lmx-model` contract types   |
//!
//! The socket is open to every local user; `lmxd` decides what each caller may do from the peer
//! credentials of the connection. The crate has no Solti dependency, so `lmx` stays light.
#![forbid(unsafe_code)]

mod client;
mod convert;

pub use client::connect;
pub use convert::{InvalidAnswer, reserve_outcome};
pub use tonic;

/// Path of the socket inside a booted guest.
pub const SOCKET_PATH: &str = "/run/lmx/lmx.sock";

/// Messages and services of `lmx.v1`, generated from `proto/lmx/v1/owner.proto`.
#[allow(
    missing_docs,
    unreachable_pub,
    clippy::missing_docs_in_private_items,
    clippy::doc_markdown
)]
pub mod proto {
    tonic::include_proto!("lmx.v1");
}
