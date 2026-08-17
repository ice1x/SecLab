//! `vb-project` — project serialization for VST_bomb.
//!
//! A [`ProjectDoc`] is the on-disk representation of a patch: the structural graph
//! (nodes with their ports, plus edges) in a versioned, `serde`-friendly form. **RON**
//! is the source-of-truth format.
//!
//! Loading a file is three steps, each its own module: parse ([`io`]), bring the schema
//! up to date ([`migrate`]), then check it against the graph's rules ([`validate`]).
//! [`open_path`] runs all three; [`load_from_path`] stops after parsing when a tool needs
//! to inspect a file as written.

pub mod doc;
pub mod error;
pub mod io;
pub mod migrate;
pub mod prompt;
pub mod validate;

pub use doc::{NodeEntry, ProjectDoc, FORMAT_VERSION, MIN_SUPPORTED_VERSION};
pub use error::ProjectError;
pub use io::{from_ron, load_from_path, open_path, open_ron, save_to_path, to_ron};
pub use migrate::migrate_to_current;
pub use validate::Issue;
