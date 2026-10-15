//! The domain layer: plain data and rules of the IDE features, and the
//! ports (traits) through which use cases reach the outside world. Nothing
//! here knows about GPUI, processes, files or the index's storage.
//!
//! Layers (dependencies point inward only):
//! `ui` → `usecase` → `domain` ← `infra`.

pub mod android_resources;
pub mod build;
pub mod completion;
pub mod debugger;
pub mod dependency_versions;
pub mod diagnostics;
pub mod expression;
pub mod imports;
pub mod jdk;
pub mod local_history;
pub mod logcat;
pub mod rename;
pub mod run;
pub mod testing;
pub mod todo;
pub mod sync;
pub mod version_catalog;
