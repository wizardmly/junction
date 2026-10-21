//! Use cases: one per IDE feature, driving the domain through its ports.
//! They know nothing about GPUI or how a port is implemented.

pub mod merged_manifest;
pub mod android_resources;
pub mod import_class;
pub mod clean_up_files;
pub mod translations;
pub mod check_syntax;
pub mod compile_on_save;
pub mod debug_session;
pub mod hot_swap;
pub mod evaluate;
pub mod dependency_updates;
pub mod local_history;
pub mod logcat;
pub mod run_app;
pub mod rename_symbol;
pub mod update_calls;
pub mod project_edits;
pub mod run_tests;
pub mod complete_code;
pub mod sync_project;
pub mod project_structure;
