# Ironclad Windows Installer
#
# Interactive usage (recommended):
#   irm https://roboticus.ai/install.ps1 | iex
#
# Non-interactive (skip all prompts):
#   $env:IRONCLAD_YES=1; irm https://roboticus.ai/install.ps1 | iex
#
# Environment variables:
#   IRONCLAD_VERSION  Specific version to install (default: latest)
#   IRONCLAD_NO_INIT  Skip "ironclad init" after install (set to 1)
#   IRONCLAD_YES      Skip all confirmation prompts (set to 1)

$ErrorActionPreference = "Stop"

$Crate = "ironclad-server"
$Version = $env:IRONCLAD_VERSION
$AutoYes = $env:IRONCLAD_YES -eq "1"
$SkipInit = $env:IRONCLAD_NO_INIT -eq "1"

function Test-Command {
    param([Parameter(Mandatory = $true)][string]$Name)
    return $null -ne (Get-Command $Name -ErrorAction SilentlyContinue)
}

function Write-Step {
    param([Parameter(Mandatory = $true)][string]$Message)
    Write-Host ""
    Write-Host "  > $Message" -ForegroundColor Green
}

function Write-Info {
    param([Parameter(Mandatory = $true)][string]$Message)
    Write-Host "    $Message"
}

function Write-Warn {
    param([Parameter(Mandatory = $true)][string]$Message)
    Write-Host "  ! $Message" -ForegroundColor Yellow
}

function Confirm-Step {
    param(
        [Parameter(Mandatory = $true)][string]$Prompt,
        [string]$Default = "Y"
    )

    if ($AutoYes) {
        return $true
    }

    $suffix = if ($Default -eq "Y") { "[Y/n]" } else { "[y/N]" }
    $answer = Read-Host "$Prompt $suffix"
    if ([string]::IsNullOrWhiteSpace($answer)) {
        $answer = $Default
    }
    return $answer -match "^(y|yes)$"
}

function Refresh-Path {
    $machinePath = [Environment]::GetEnvironmentVariable("Path", "Machine")
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $env:Path = "$machinePath;$userPath"
}

function Test-MsvcBuildToolsInstalled {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
    if (-not (Test-Path $vswhere)) {
        return $false
    }
    $installationPath = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath 2>$null
    return -not [string]::IsNullOrWhiteSpace($installationPath)
}

if (-not $IsWindows) {
    throw "This installer is for Windows only."
}

Write-Host ""
Write-Host "        IRONCLAD" -ForegroundColor Cyan
Write-Host "  Windows installer (PowerShell)"
Write-Host ""

$versionDisplay = if ([string]::IsNullOrWhiteSpace($Version)) { "latest" } else { $Version }
Write-Step "Installation plan"
Write-Info "1. Check prerequisites (Rust + C toolchain)"
Write-Info "2. Install Ironclad from crates.io"
Write-Info "3. Verify install and optionally initialize workspace"
Write-Info "Version: $versionDisplay"

if (-not (Confirm-Step "Proceed with installation?")) {
    Write-Warn "Installation cancelled by user."
    exit 0
}

if (-not (Test-Command "cargo")) {
    Write-Step "Rust toolchain not found"
    Write-Info "Installing rustup and stable toolchain..."

    $rustupInstaller = Join-Path $env:TEMP "rustup-init.exe"
    Invoke-WebRequest "https://win.rustup.rs" -OutFile $rustupInstaller
    & $rustupInstaller -y --default-toolchain stable | Out-Null

    Refresh-Path
}

if (-not (Test-Command "cargo")) {
    throw "Cargo is still not available after Rust installation."
}

Write-Step "Checking C toolchain"
if (Test-Command "cl") {
    Write-Info "MSVC compiler found (cl.exe)."
}
elseif (Test-MsvcBuildToolsInstalled) {
    Write-Info "MSVC build tools are installed (cl.exe not in current shell PATH)."
}
else {
    Write-Warn "MSVC build tools not detected."
    if ((Test-Command "winget") -and (Confirm-Step "Install Visual Studio Build Tools now?")) {
        Write-Info "Installing Build Tools via winget (this can take several minutes)..."
        & winget install -e --id Microsoft.VisualStudio.2022.BuildTools --accept-package-agreements --accept-source-agreements
        if ($LASTEXITCODE -ne 0) {
            Write-Warn "Build Tools installation command failed. You can retry manually:"
            Write-Warn "winget install -e --id Microsoft.VisualStudio.2022.BuildTools"
        }
        else {
            Write-Info "Build Tools install command completed."
            Write-Info "You may need to open a new shell after installation."
        }
    }
    else {
        Write-Warn "If compilation fails, install Build Tools manually:"
        Write-Warn "winget install -e --id Microsoft.VisualStudio.2022.BuildTools"
    }
}

Write-Step "Installing Ironclad"
$installArgs = @("install", $Crate)
if (-not [string]::IsNullOrWhiteSpace($Version)) {
    $installArgs += @("--version", $Version)
}
& cargo @installArgs

Refresh-Path
$ironcladCmd = Get-Command "ironclad" -ErrorAction SilentlyContinue
if ($null -eq $ironcladCmd) {
    $fallback = Join-Path $env:USERPROFILE ".cargo\bin\ironclad.exe"
    if (-not (Test-Path $fallback)) {
        throw "Installed binary not found on PATH or at $fallback"
    }
    $ironcladPath = $fallback
}
else {
    $ironcladPath = $ironcladCmd.Source
}

Write-Step "Verifying installation"
Write-Info "Binary: $ironcladPath"
& $ironcladPath version

if (-not $SkipInit) {
    Write-Step "Initialize workspace"
    if (Confirm-Step "Initialize workspace now?") {
        & $ironcladPath init
        Write-Info "Workspace ready."
    }
}

Write-Host ""
Write-Host "  [OK] Ironclad installed successfully!" -ForegroundColor Green
Write-Host ""
Write-Host "  Next steps:"
Write-Host "    ironclad setup"
Write-Host "    ironclad serve"
Write-Host "    ironclad --help"
Write-Host ""
