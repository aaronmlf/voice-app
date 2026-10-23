//! Windows WASAPI adapter, including an explicit route into the separately installed
//! VB-CABLE or a compatible legacy first-party pair. This backend never installs
//! a driver or creates audio endpoints.
//! The prepared PCM callbacks live separately from device enumeration and stream ownership.

#[cfg(any(target_os = "windows", test))]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
mod rt;

#[cfg(any(target_os = "windows", test))]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
mod identity;

#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "windows")]
pub use windows::{
    AudioHandle, DesktopCaptureHandle, list_devices, resolve_config, start, start_desktop,
    virtual_microphone_status, virtual_microphone_status_for,
};
