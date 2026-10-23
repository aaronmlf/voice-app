//! Windows WASAPI adapter. This backend does not install or create a virtual microphone.
//! The prepared PCM callbacks live separately from device enumeration and stream ownership.

#[cfg(any(target_os = "windows", test))]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
mod rt;

#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "windows")]
pub use windows::{
    AudioHandle, DesktopCaptureHandle, list_devices, resolve_config, start, start_desktop,
};
