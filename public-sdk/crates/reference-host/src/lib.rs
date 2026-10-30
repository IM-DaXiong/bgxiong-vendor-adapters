//! Independent development host. Only explicitly created SDK workspaces are
//! opened; no application database, runtime configuration or ambient credentials.
pub mod cancel;
pub mod checkpoint;
pub mod config;
pub mod credential;
pub mod dispatch;
pub mod events;
pub mod http;
pub mod http_stream;
pub mod media;
pub mod policy;
pub mod session;
pub use dispatch::ReferenceHost;

pub mod profile;

pub mod before_send;
