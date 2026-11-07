pub mod clipboard_cmd;
pub mod favorites_cmd;
pub mod history_cmd;
pub mod hotkey_cmd;
pub mod settings_cmd;
pub mod system_cmd;
pub mod thumbnail_cmd;

// Re-export all commands for convenience in main.rs if needed,
// though tauri usually expects them to be referenced via module path in generate_handler!
pub use clipboard_cmd::*;
pub use favorites_cmd::*;
pub use history_cmd::*;
pub use hotkey_cmd::*;
pub use settings_cmd::*;
pub use system_cmd::*;
pub use thumbnail_cmd::*;
