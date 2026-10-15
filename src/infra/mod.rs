//! Adapters implementing the domain's ports over the outside world: the
//! code index, Gradle, adb.

pub mod adb_device;
pub mod adb_logcat;
pub mod gradle;
pub mod gradle_sync;
pub mod local_history_store;
pub mod index_code_model;
pub mod tree_sitter_syntax;
