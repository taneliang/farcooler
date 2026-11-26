//! Shared daemon messages, launch inputs and CLI helpers.

pub mod board;
pub mod caller;
pub mod coding_pins;
pub mod landing_templates;
pub mod launch;
pub mod layout;
pub mod opening;
pub mod path_routes;
pub mod paths;
pub mod report;
#[cfg(target_os = "macos")]
pub mod standalone_macos;
pub mod workspace_paths;
