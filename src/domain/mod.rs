//! The domain layer: plain data and rules of the IDE features, and the
//! ports (traits) through which use cases reach the outside world. Nothing
//! here knows about GPUI, processes, files or the index's storage.
//!
//! Layers (dependencies point inward only):
//! `ui` → `usecase` → `domain` ← `infra`.

pub mod android_resources;
pub mod ansi;
pub mod excluded_folders;
pub mod build;
pub mod completion;
pub mod debugger;
pub mod dependency_versions;
pub mod diagnostics;
pub mod expression;
pub mod generate;
pub mod hotswap;
pub mod imports;
pub mod line_edit;
pub mod navigation_history;
pub mod project_files;
pub mod editor_session;
pub mod intentions;
pub mod introduce_variable;
pub mod live_templates;
pub mod catalog_migration;
pub mod coverage;
pub mod dependencies;
pub mod surround;
pub mod jdk;
pub mod local_history;
pub mod logcat;
pub mod logcat_filter;
pub mod overrides;
pub mod password_safe;
pub mod project_structure;
pub mod parameter_info;
pub mod reformat;
pub mod rename;
pub mod run;
pub mod signing;
pub mod testing;
pub mod translations;
pub mod todo;
pub mod sync;
pub mod vector_drawable;
pub mod drawable;
pub mod change_signature;
pub mod class_stub;
pub mod console_links;
pub mod extract_method;
pub mod manifest_merge;
pub mod jdk_feed;
pub mod introduce_parameter;
pub mod inline_function;
pub mod version_catalog;
