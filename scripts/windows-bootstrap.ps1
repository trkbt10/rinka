[CmdletBinding()]
param(
    [string]$RustToolchain = "1.97.0-x86_64-pc-windows-msvc",
    [string]$BuildToolsPath = "C:\BuildTools",
    [string]$EvidencePath = "target\windows-bootstrap.json"
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw "Windows bootstrap requires an elevated PowerShell session"
}
if ($env:PROCESSOR_ARCHITECTURE -ne "AMD64") {
    throw "Windows bootstrap requires an AMD64 host"
}

[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$temporaryDirectory = Join-Path $env:TEMP "rinka-bootstrap"
New-Item -ItemType Directory -Path $temporaryDirectory -Force | Out-Null

$vsWhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
$requiredComponents = @(
    "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
    "Microsoft.VisualStudio.Component.Windows11SDK.26100"
)

function Get-NativeBuildToolsProperty {
    param([string]$Property)
    if (-not (Test-Path $vsWhere -PathType Leaf)) {
        return $null
    }
    $arguments = @("-latest", "-products", "*", "-requires")
    $arguments += $requiredComponents
    $arguments += @("-property", $Property)
    $installation = & $vsWhere @arguments | Select-Object -First 1
    if ([string]::IsNullOrWhiteSpace([string]$installation)) {
        return $null
    }
    return $installation.Trim()
}

function Find-NativeBuildTools {
    return Get-NativeBuildToolsProperty -Property "installationPath"
}

$nativeBuildTools = Find-NativeBuildTools
if ($null -eq $nativeBuildTools) {
    $bootstrapper = Join-Path $temporaryDirectory "vs_buildtools.exe"
    Invoke-WebRequest `
        -Uri "https://aka.ms/vs/17/release/vs_buildtools.exe" `
        -OutFile $bootstrapper `
        -UseBasicParsing
    $arguments = @(
        "--installPath", $BuildToolsPath,
        "--add", "Microsoft.VisualStudio.Workload.VCTools",
        "--add", $requiredComponents[0],
        "--add", $requiredComponents[1],
        "--quiet",
        "--wait",
        "--norestart",
        "--nocache"
    )
    $installer = Start-Process `
        -FilePath $bootstrapper `
        -ArgumentList $arguments `
        -Wait `
        -PassThru
    if ($installer.ExitCode -eq 3010) {
        throw "Visual Studio Build Tools requires a reboot; restart Windows and rerun this script"
    }
    if ($installer.ExitCode -ne 0) {
        throw "Visual Studio Build Tools installer exited with code $($installer.ExitCode)"
    }
    $nativeBuildTools = Find-NativeBuildTools
    if ($null -eq $nativeBuildTools) {
        throw "Required MSVC and Windows SDK 26100 components were not discovered after installation"
    }
}

$cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
$rustup = Join-Path $cargoBin "rustup.exe"
if (-not (Test-Path $rustup -PathType Leaf)) {
    $rustupInit = Join-Path $temporaryDirectory "rustup-init.exe"
    Invoke-WebRequest `
        -Uri "https://win.rustup.rs/x86_64" `
        -OutFile $rustupInit `
        -UseBasicParsing
    $rustupProcess = Start-Process `
        -FilePath $rustupInit `
        -ArgumentList @("-y", "--default-toolchain", "none", "--profile", "minimal") `
        -Wait `
        -PassThru
    if ($rustupProcess.ExitCode -ne 0) {
        throw "rustup-init exited with code $($rustupProcess.ExitCode)"
    }
}

$env:Path = "$cargoBin;$env:Path"
& $rustup toolchain install $RustToolchain --profile minimal --component rustfmt --component clippy
if ($LASTEXITCODE -ne 0) {
    throw "rustup toolchain install failed with code $LASTEXITCODE"
}
& $rustup default $RustToolchain
if ($LASTEXITCODE -ne 0) {
    throw "rustup default failed with code $LASTEXITCODE"
}

$sdkRoots = @(Get-ChildItem `
    -Path "${env:ProgramFiles(x86)}\Windows Kits\10\Lib\10.0.26100.*" `
    -Directory `
    -ErrorAction SilentlyContinue | Sort-Object Name)
if ($sdkRoots.Count -eq 0) {
    throw "Windows SDK 26100 library directory was not found"
}

$rustcVersion = (& rustc -Vv) -join "`n"
if ($LASTEXITCODE -ne 0) {
    throw "rustc version probe failed with code $LASTEXITCODE"
}
$cargoVersion = (& cargo -V) -join "`n"
if ($LASTEXITCODE -ne 0) {
    throw "cargo version probe failed with code $LASTEXITCODE"
}
$buildToolsVersion = Get-NativeBuildToolsProperty -Property "installationVersion"
if ([string]::IsNullOrWhiteSpace([string]$buildToolsVersion)) {
    throw "Visual Studio Build Tools version was not discovered"
}
$operatingSystem = Get-CimInstance -ClassName Win32_OperatingSystem
$result = [ordered]@{
    schema = 1
    platform = "Windows Server 2025 Desktop Experience"
    captured_at_utc = [DateTime]::UtcNow.ToString("o")
    operating_system = [ordered]@{
        caption = $operatingSystem.Caption
        version = $operatingSystem.Version
        build = $operatingSystem.BuildNumber
    }
    build_tools = $nativeBuildTools
    build_tools_version = $buildToolsVersion
    required_components = $requiredComponents
    windows_sdk = @($sdkRoots | ForEach-Object { $_.FullName })
    rust_toolchain = $RustToolchain
    rustc = $rustcVersion
    cargo = $cargoVersion
    result = "PASS"
}
$evidenceDirectory = Split-Path -Parent $EvidencePath
if (-not [string]::IsNullOrWhiteSpace($evidenceDirectory)) {
    New-Item -ItemType Directory -Path $evidenceDirectory -Force | Out-Null
}
$result | ConvertTo-Json -Depth 5 | Set-Content -Path $EvidencePath -Encoding utf8
$result | ConvertTo-Json -Depth 5
