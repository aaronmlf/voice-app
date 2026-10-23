//! Platform choice stays outside the shared contracts and DSP.
#[cfg(target_os = "linux")]
pub use voice_pipewire::{
    AudioHandle, DesktopCaptureHandle, list_devices, resolve_config, start, start_desktop,
};
#[cfg(windows)]
pub use voice_wasapi::{
    AudioHandle, DesktopCaptureHandle, list_devices, resolve_config, start, start_desktop,
};

#[cfg(target_os = "linux")]
pub const BACKEND_NAME: &str = "pipewire";
#[cfg(windows)]
pub const BACKEND_NAME: &str = "wasapi";

/// The Linux adapter publishes its own virtual source. Windows availability is probed at runtime.
pub const VIRTUAL_MICROPHONE: bool = cfg!(target_os = "linux");
pub const DESKTOP_CAPTURE: bool = true;

#[cfg(windows)]
pub fn virtual_microphone_status() -> anyhow::Result<voice_core::VirtualMicrophoneStatus> {
    voice_wasapi::virtual_microphone_status()
}

#[cfg(target_os = "linux")]
pub fn virtual_microphone_status() -> anyhow::Result<voice_core::VirtualMicrophoneStatus> {
    Ok(voice_core::VirtualMicrophoneStatus {
        available: true,
        ..Default::default()
    })
}

#[cfg(not(any(target_os = "linux", windows)))]
compile_error!("voice-engine currently supports Linux/PipeWire and Windows/WASAPI only");
