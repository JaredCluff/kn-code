//! Plugin SDK for kn-code
//!
//! This crate provides macros and helpers for writing kn-code plugins.

pub mod prelude {
    pub use serde::{Deserialize, Serialize};
    pub use serde_json;
}

/// Define a plugin
#[macro_export]
macro_rules! plugin {
    ($name:ident) => {
        pub struct $name;
    };
}

/// Define a plugin tool
#[macro_export]
macro_rules! plugin_tool {
    (name = $name:expr, description = $desc:expr) => {
        #[allow(dead_code)]
        fn tool_name() -> &'static str {
            $name
        }

        #[allow(dead_code)]
        fn tool_description() -> &'static str {
            $desc
        }
    };
}

/// Define a plugin hook
#[macro_export]
macro_rules! plugin_hook {
    ($hook_type:ident, tool = $tool:expr) => {
        #[allow(dead_code)]
        fn hook_tool_filter() -> &'static str {
            $tool
        }
    };
}
