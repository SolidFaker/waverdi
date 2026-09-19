//! The background loading runtime moved to [`crate::session`]: it is format
//! independent and must not know the App. This module only re-exports the
//! types the App drives, so the existing `crate::app::load::…` paths keep
//! working.

pub use crate::session::{LoadEvent, LoadJob};
