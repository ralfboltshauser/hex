//! Local recording recovery. This module owns persistence, quotas and deletion;
//! capture, inference and output remain owned by their existing modules.
mod actions;
mod client;
mod faults;
mod format;
mod fs;
mod retry;
mod store;
#[cfg(test)]
mod tests;

pub use actions::execute;
pub use client::{Client, Session};
pub use format::{CaptureState, Delivery, Failure, SessionId, Snapshot};
pub use store::{AudioReader, Entry};
