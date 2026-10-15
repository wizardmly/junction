//! Use cases: one per IDE feature, driving the domain through its ports.
//! They know nothing about GPUI or how a port is implemented.

pub mod check_syntax;
pub mod compile_on_save;
pub mod run_app;
pub mod complete_code;
pub mod sync_project;
