//! Code indexing: a built-in tree-sitter symbol index, language servers, and
//! a bridge index linking code across languages (JNI, FFI, C ABI, Swift and
//! Objective-C).

pub mod lang;
pub mod symbols;
pub mod bridge;
pub mod store;
pub mod nav;
