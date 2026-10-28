//! Code indexing: a built-in tree-sitter symbol index, language servers, and
//! a bridge index linking code across languages (JNI, FFI, C ABI, Swift and
//! Objective-C).

pub mod lang;
pub mod symbols;
pub mod bridge;
pub mod store;
pub mod libraries;
pub mod nav;
pub mod groovy_scripts;
pub mod hierarchy;
pub mod dependencies;
pub mod locals;
pub mod introduce;
pub mod call_hierarchy;
pub mod inline;
pub mod inline_field;
pub mod intentions;
pub mod import_function;
pub mod inline_function;
pub mod safe_delete;
pub mod unwrap;
pub mod surround;
pub mod change_signature;
pub mod extract_method;
pub mod introduce_parameter;
pub mod method_breakpoint;
pub mod selection;
pub mod generate;
pub mod overrides;
pub mod special;
pub mod lsp;
pub mod service;
pub mod text_search;
