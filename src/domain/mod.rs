//! The domain layer: plain data and rules of the IDE features, and the
//! ports (traits) through which use cases reach the outside world. Nothing
//! here knows about GPUI, processes, files or the index's storage.
//!
//! Layers (dependencies point inward only):
//! `ui` → `usecase` → `domain` ← `infra`.

pub mod build;
pub mod completion;
pub mod diagnostics;
pub mod imports;
