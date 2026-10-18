//! Adapters implementing the domain's ports over the outside world: the
//! code index, Gradle, adb.

pub mod adb_device;
pub mod breakpoint_store;
pub mod editor_session_store;
pub mod run_config_store;
pub mod signing_store;
pub mod jdk_download;
pub mod sources_download;
pub mod adb_logcat;
pub mod logcat_filter_store;
pub mod catalog_files;
pub mod gradle;
pub mod gradle_sync;
pub mod gradle_tests;
pub mod jdk_discovery;
pub mod project_files;
pub mod res_files;
pub mod jdwp_client;
pub mod local_history_store;
pub mod maven_versions;
pub mod resource_scanner;
pub mod index_code_model;
pub mod tree_sitter_syntax;
