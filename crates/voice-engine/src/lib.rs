//! Shared session and command service. See protocol for the GUI/CLI/MCP contract.
pub mod client;
pub mod protocol;
mod service;
pub use service::{Paths, ServiceHandle, spawn_service};

/// Enumerate devices without opening a microphone or starting a session.
pub fn probe_devices() -> anyhow::Result<Vec<voice_core::DeviceInfo>> {
    voice_pipewire::list_devices()
}
