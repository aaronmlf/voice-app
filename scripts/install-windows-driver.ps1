# SPDX-License-Identifier: MIT OR Apache-2.0
#Requires -Version 7.0
#Requires -RunAsAdministrator
[CmdletBinding(SupportsShouldProcess, ConfirmImpact = 'High')]
param(
    [Parameter(Mandatory, ParameterSetName = 'Install')]
    [string] $PackageDirectory,
    [Parameter(Mandatory, ParameterSetName = 'Remove')]
    [switch] $Remove
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows -or -not [Environment]::Is64BitProcess -or
    [Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne [Runtime.InteropServices.Architecture]::X64) {
    throw 'This operation requires native x64 Windows.'
}
if ([Environment]::OSVersion.Version.Build -lt 22000) {
    throw 'The initial driver targets Windows 11 x64. Windows 10 is not qualified.'
}
$instance = 'ROOT\VoiceAppVirtualAudio\0000'
$hardware = 'ROOT\VoiceAppVirtualAudio'
$mediaClass = [Guid]::new('4d36e96c-e325-11ce-bfc1-08002be10318')

# Root-enumerated instance ID is deliberately fixed. DevCon's generated ROOT\MEDIA
# instance cannot satisfy the adapter topology identity used by the audio backend.
function Get-OwnedDevice {
    $device = Get-PnpDevice -InstanceId $instance -ErrorAction SilentlyContinue
    if ($null -eq $device) { return $null }
    $ids = (Get-PnpDeviceProperty -InstanceId $instance -KeyName 'DEVPKEY_Device_HardwareIds').Data
    $class = (Get-PnpDeviceProperty -InstanceId $instance -KeyName 'DEVPKEY_Device_ClassGuid').Data
    if ($hardware -notin @($ids) -or [Guid]$class -ne $mediaClass) {
        throw 'The fixed instance belongs to another device; no change was made.'
    }
    return $device
}

if ($Remove) {
    $device = Get-OwnedDevice
    if ($null -eq $device) { Write-Output 'Voice App virtual audio device is not installed.'; return }
    if ($PSCmdlet.ShouldProcess($instance, 'Remove only the Voice App virtual audio device')) {
        & pnputil.exe /remove-device $instance
        if ($LASTEXITCODE -ne 0) { throw "Device removal failed: $LASTEXITCODE" }
        Write-Output 'Device removed. Driver-store package and application library were retained.'
    }
    return
}

$directory = [IO.Path]::GetFullPath($PackageDirectory)
if ($directory.StartsWith('\\')) { throw 'Copy the signed package to a local directory first.' }
$inf = Join-Path $directory 'VoiceAppVirtualAudio.inf'
$sys = Join-Path $directory 'VoiceAppVirtualAudio.sys'
$catalog = Join-Path $directory 'VoiceAppVirtualAudio.cat'
foreach ($path in @($directory, $inf, $sys, $catalog)) {
    $item = Get-Item -LiteralPath $path -Force
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw 'Reparse points are not accepted for driver installation.'
    }
}
$signature = Get-AuthenticodeSignature -LiteralPath $catalog
if ($signature.Status -ne 'Valid') {
    throw 'A valid signed driver catalog is required. The unsigned CI artifact is for development and cannot be installed by this script. No certificates or boot protections were changed.'
}
if ((Get-Item -LiteralPath $inf).Length -gt 131072) { throw 'Driver INF exceeds the size limit.' }
$content = [IO.File]::ReadAllText($inf)
if ($content -notmatch '(?im)^CatalogFile\s*=\s*VoiceAppVirtualAudio\.cat\s*$' -or
    $content -notmatch '(?im)^.*=\s*[^,\r\n]+,\s*ROOT\\VoiceAppVirtualAudio\s*$' -or
    $content -notmatch '(?im)^ServiceBinary\s*=\s*%13%\\VoiceAppVirtualAudio\.sys\s*$') {
    throw 'The driver package does not match the Voice App hardware and service contract.'
}
$existing = Get-OwnedDevice
if (-not $PSCmdlet.ShouldProcess($instance, "Stage and install the signed Voice App package from $directory")) { return }

# Windows PnP verifies catalog membership and driver trust during staging/install.
# No force-unsigned, certificate import, test-signing or default-audio change is used.
& pnputil.exe /add-driver $inf
if ($LASTEXITCODE -ne 0) { throw "Signed package staging failed: $LASTEXITCODE" }

if (-not ('VoiceAppDriverSetup.Native' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
namespace VoiceAppDriverSetup {
    [StructLayout(LayoutKind.Sequential)] public struct DeviceInfo {
        public uint Size; public Guid ClassGuid; public uint DevInst; public UIntPtr Reserved;
    }
    public static class Native {
        [DllImport("setupapi.dll", SetLastError=true)]
        static extern IntPtr SetupDiCreateDeviceInfoList(ref Guid guid, IntPtr parent);
        [DllImport("setupapi.dll", CharSet=CharSet.Unicode, SetLastError=true)]
        static extern bool SetupDiCreateDeviceInfoW(IntPtr set, string name, ref Guid guid,
            string description, IntPtr parent, uint flags, ref DeviceInfo info);
        [DllImport("setupapi.dll", SetLastError=true)]
        static extern bool SetupDiSetDeviceRegistryPropertyW(IntPtr set, ref DeviceInfo info,
            uint property, byte[] value, uint bytes);
        [DllImport("setupapi.dll", SetLastError=true)]
        static extern bool SetupDiCallClassInstaller(uint operation, IntPtr set, ref DeviceInfo info);
        [DllImport("setupapi.dll", SetLastError=true)]
        static extern bool SetupDiDestroyDeviceInfoList(IntPtr set);
        [DllImport("newdev.dll", CharSet=CharSet.Unicode, SetLastError=true)]
        static extern bool UpdateDriverForPlugAndPlayDevicesW(IntPtr parent, string hardware,
            string inf, uint flags, out bool reboot);
        static void Check(bool ok) { if (!ok) throw new Win32Exception(Marshal.GetLastWin32Error()); }
        public static bool Install(string inf, bool create) {
            Guid media = new Guid("4d36e96c-e325-11ce-bfc1-08002be10318");
            IntPtr set = SetupDiCreateDeviceInfoList(ref media, IntPtr.Zero);
            if (set == new IntPtr(-1)) throw new Win32Exception(Marshal.GetLastWin32Error());
            DeviceInfo info = new DeviceInfo { Size = (uint)Marshal.SizeOf<DeviceInfo>() };
            bool registered = false;
            try {
                if (create) {
                    Check(SetupDiCreateDeviceInfoW(set, @"ROOT\VoiceAppVirtualAudio\0000", ref media,
                        "Voice App Virtual Audio", IntPtr.Zero, 0, ref info));
                    byte[] hardware = System.Text.Encoding.Unicode.GetBytes(@"ROOT\VoiceAppVirtualAudio" + "\0\0");
                    Check(SetupDiSetDeviceRegistryPropertyW(set, ref info, 1, hardware, (uint)hardware.Length));
                    Check(SetupDiCallClassInstaller(0x19, set, ref info)); // DIF_REGISTERDEVICE
                    registered = true;
                }
                bool reboot;
                Check(UpdateDriverForPlugAndPlayDevicesW(IntPtr.Zero, @"ROOT\VoiceAppVirtualAudio", inf, 0, out reboot));
                return reboot;
            } catch {
                // Roll back only the devnode created by this attempt. Existing devices
                // and the staged signed package are retained after an update failure.
                if (registered) SetupDiCallClassInstaller(5, set, ref info); // DIF_REMOVE
                throw;
            } finally { SetupDiDestroyDeviceInfoList(set); }
        }
    }
}
'@
}
$reboot = [VoiceAppDriverSetup.Native]::Install($inf, $null -eq $existing)
$installed = Get-OwnedDevice
if ($null -eq $installed) { throw 'Windows did not expose the expected fixed device instance.' }
if ($installed.Status -ne 'OK' -and -not $reboot) {
    throw "The virtual audio device is not ready: $($installed.Status). Check Windows driver diagnostics."
}
Write-Output "Voice App driver installation requested. Restart required: $reboot."
Write-Output 'Open Voice App, explicitly enable its virtual output, then select Voice App Virtual Microphone in the destination application.'
