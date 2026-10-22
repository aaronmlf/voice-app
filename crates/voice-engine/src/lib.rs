//! Shared session and command service. See protocol for the GUI/CLI/MCP contract.
mod audio;
pub mod client;
mod platform;
pub mod protocol;
mod service;
pub use audio::{BACKEND_NAME, DESKTOP_CAPTURE, VIRTUAL_MICROPHONE};
pub use service::{Paths, ServiceHandle, persist_private_file, spawn_service};

/// Enumerate devices without opening a microphone or starting a session.
pub fn probe_devices() -> anyhow::Result<Vec<voice_core::DeviceInfo>> {
    audio::list_devices()
}
