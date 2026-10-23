# SPDX-License-Identifier: MIT OR Apache-2.0
[CmdletBinding()]
param([string] $OutputDirectory = (Join-Path $PSScriptRoot '../../target/windows-driver'))
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$version = '10.0.28000.2526'
if (-not $IsWindows) { throw 'The WDK build requires native Windows, MSBuild and Visual Studio 2026.' }
$output = [IO.Path]::GetFullPath($OutputDirectory)
$packages = Join-Path $output 'packages'
$bin = Join-Path $output 'bin'
$package = Join-Path $output 'unsigned-package'
$metadataFiles = @('LICENSE-MS-PL.txt', 'LICENSE-MIT', 'LICENSE-APACHE', 'upstream-manifest.json', 'PROVENANCE.md', 'README.md', 'identity.json')
$artifactFiles = @('VoiceAppVirtualAudio.sys', 'VoiceAppVirtualAudio.inf', 'VoiceAppVirtualAudio.cat') + $metadataFiles
$nuget = (Get-Command nuget.exe -ErrorAction Stop).Source
$msbuild = (Get-Command msbuild.exe -ErrorAction Stop).Source
New-Item -ItemType Directory -Force -Path $output, $packages, $package | Out-Null
foreach ($existing in @(Get-ChildItem -LiteralPath $package -Force)) {
    if ($existing.PSIsContainer -or $existing.Name -notin $artifactFiles -or ($existing.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "Unexpected content in candidate directory; inspect it before building: $($existing.Name)"
    }
}
& $nuget restore (Join-Path $PSScriptRoot 'packages.config') -PackagesDirectory $packages -Source 'https://api.nuget.org/v3/index.json' -NonInteractive
if ($LASTEXITCODE -ne 0) { throw "Pinned WDK/SDK restore failed: $LASTEXITCODE" }
$kitPackages = @('Microsoft.Windows.SDK.CPP', 'Microsoft.Windows.SDK.CPP.x64', 'Microsoft.Windows.WDK.x64')
$packageHashes = @()
foreach ($id in $kitPackages) {
    $nupkg = Join-Path $packages "$id.$version/$id.$version.nupkg"
    if (-not (Test-Path -LiteralPath $nupkg -PathType Leaf)) { throw "Missing pinned package: $id" }
    & $nuget verify -Signatures $nupkg
    if ($LASTEXITCODE -ne 0) { throw "Package signature verification failed: $id" }
    $packageHashes += @{ id = $id; version = $version; sha256 = (Get-FileHash -LiteralPath $nupkg -Algorithm SHA256).Hash.ToLowerInvariant() }
}
& $msbuild (Join-Path $PSScriptRoot 'VoiceAppVirtualAudio.vcxproj') /m:1 /p:Configuration=Release /p:Platform=x64 /p:SignMode=Off /p:DriverSign=Off "/p:VoiceAppWdkPackages=$packages" "/p:VoiceAppBuildRoot=$output" /nologo /verbosity:minimal
if ($LASTEXITCODE -ne 0) { throw "WDK build failed: $LASTEXITCODE" }
$sys = Get-Item -LiteralPath (Join-Path $bin 'VoiceAppVirtualAudio.sys') -ErrorAction Stop
if ($sys.PSIsContainer -or ($sys.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Unsafe SYS build output.' }
Copy-Item -LiteralPath $sys.FullName -Destination (Join-Path $package $sys.Name) -Force
# StampInf may emit into IntDir even though the linked SYS uses OutDir.
# Accept only these two exact build-owned paths, never a recursive search.
$infCandidates = @(
    (Join-Path $bin 'VoiceAppVirtualAudio.inf'),
    (Join-Path $output 'obj/VoiceAppVirtualAudio.inf')
)
$infFiles = @()
foreach ($path in $infCandidates) {
    if (Test-Path -LiteralPath $path) {
        $file = Get-Item -LiteralPath $path -ErrorAction Stop
        if ($file.PSIsContainer -or ($file.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Unsafe INF build output.' }
        $infFiles += $file
    }
}
if ($infFiles.Count -eq 0) { throw 'Stamped VoiceAppVirtualAudio.inf is missing from bin and obj.' }
$infHash = (Get-FileHash -LiteralPath $infFiles[0].FullName -Algorithm SHA256).Hash
foreach ($file in $infFiles) {
    if ((Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash -ne $infHash) {
        throw 'Differing stamped INF files exist in bin and obj; refusing an ambiguous package.'
    }
}
Copy-Item -LiteralPath $infFiles[0].FullName -Destination (Join-Path $package 'VoiceAppVirtualAudio.inf') -Force
$wdk = Join-Path $packages "Microsoft.Windows.WDK.x64.$version"
$infverif = @(Get-ChildItem -LiteralPath $wdk -Filter InfVerif.exe -File -Recurse | Where-Object { $_.FullName -match '[\\/]x64[\\/]' } | Select-Object -First 1)
$inf2cat = @(Get-ChildItem -LiteralPath $wdk -Filter Inf2Cat.exe -File -Recurse | Where-Object { $_.FullName -match '[\\/](x64|x86)[\\/]' } | Select-Object -First 1)
if ($infverif.Count -ne 1 -or $inf2cat.Count -ne 1) { throw 'Pinned WDK validation tools missing.' }
& ($infverif[0].FullName) /w (Join-Path $package 'VoiceAppVirtualAudio.inf')
if ($LASTEXITCODE -ne 0) { throw "InfVerif failed: $LASTEXITCODE" }
& ($inf2cat[0].FullName) "/driver:$package" /os:10_CO_X64,10_NI_X64,10_GE_X64 /uselocaltime
if ($LASTEXITCODE -ne 0) { throw "Inf2Cat failed: $LASTEXITCODE" }
$cat = Join-Path $package 'VoiceAppVirtualAudio.cat'
if (-not (Test-Path -LiteralPath $cat -PathType Leaf)) { throw 'Unsigned catalog was not generated.' }
$signatures = @()
foreach ($name in @('VoiceAppVirtualAudio.sys', 'VoiceAppVirtualAudio.cat')) {
    $signature = Get-AuthenticodeSignature -LiteralPath (Join-Path $package $name)
    if ($signature.Status -ne 'NotSigned') { throw "Unexpected signing state for $name; no credentials are permitted in this build." }
    $signatures += @{ file = $name; status = $signature.Status.ToString() }
}
foreach ($name in $metadataFiles) {
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot $name) -Destination (Join-Path $package $name) -Force
}
$report = [ordered]@{
    version = 1; target = 'Windows 11 x64'; kit_version = $version
    configuration = 'Release'; build_parallelism = 1; package_signatures = $packageHashes
    signatures = $signatures; installed = $false; kernel_execution_verified = $false
    discord_verified = $false; audacity_tenacity_verified = $false
    artifact_kind = 'unsigned-driver-development-candidate'
    files = @($artifactFiles | ForEach-Object {
        $file = Get-Item -LiteralPath (Join-Path $package $_)
        if ($file.PSIsContainer -or ($file.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Unsafe candidate entry.' }
        @{ name = $file.Name; bytes = $file.Length; sha256 = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant() }
    })
}
$report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $output 'build-report.json') -Encoding utf8NoBOM
Write-Output "Built and inspected UNSIGNED development driver: $package. No driver, certificate or audio setting was installed or changed."
