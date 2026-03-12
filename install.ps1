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
#   IRONCLAD_INSTALL_LOCKED  Use cargo --locked (default: 1, set to 0 for non-locked)

$ErrorActionPreference = "Stop"

$Crate = "ironclad-server"
$Version = $env:IRONCLAD_VERSION
$AutoYes = $env:IRONCLAD_YES -eq "1"
$SkipInit = $env:IRONCLAD_NO_INIT -eq "1"
$installLockedRaw = if ([string]::IsNullOrWhiteSpace($env:IRONCLAD_INSTALL_LOCKED)) { "1" } else { $env:IRONCLAD_INSTALL_LOCKED }
$InstallLocked = -not ($installLockedRaw -match "^(0|false|no)$")
$Apertus8BRepo = if ([string]::IsNullOrWhiteSpace($env:APERTUS_8B_REPO)) { "swiss-ai/Apertus-8B-Instruct" } else { $env:APERTUS_8B_REPO }
$Apertus70BRepo = if ([string]::IsNullOrWhiteSpace($env:APERTUS_70B_REPO)) { "swiss-ai/Apertus-70B-Instruct" } else { $env:APERTUS_70B_REPO }
$Apertus8BOllama = if ([string]::IsNullOrWhiteSpace($env:APERTUS_8B_OLLAMA)) { "apertus:8b-instruct" } else { $env:APERTUS_8B_OLLAMA }
$Apertus70BOllama = if ([string]::IsNullOrWhiteSpace($env:APERTUS_70B_OLLAMA)) { "apertus:70b-instruct" } else { $env:APERTUS_70B_OLLAMA }

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

function Get-TotalRamGb {
    try {
        $mem = (Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory
        return [math]::Floor($mem / 1GB)
    }
    catch {
        return 0
    }
}

function Get-GpuVramGb {
    try {
        $gpus = Get-CimInstance Win32_VideoController | Where-Object { $_.AdapterRAM -gt 0 }
        if ($null -eq $gpus -or $gpus.Count -eq 0) {
            return 0
        }
        $maxBytes = ($gpus | Measure-Object -Property AdapterRAM -Maximum).Maximum
        return [math]::Floor($maxBytes / 1GB)
    }
    catch {
        return 0
    }
}

function Test-HostReady {
    param([Parameter(Mandatory = $true)][string]$Host)
    switch ($Host) {
        "sglang" { return (Test-Command "sglang") }
        "vllm" { return (Test-Command "vllm") }
        "docker-model-runner" { return (Test-Command "docker") }
        "ollama" { return (Test-Command "ollama") }
        default { return $false }
    }
}

function Test-HostHasModels {
    param([Parameter(Mandatory = $true)][string]$Host)
    switch ($Host) {
        "ollama" {
            if (-not (Test-Command "ollama")) { return $false }
            try {
                $lines = (& ollama list 2>$null | Measure-Object -Line).Lines
                return ($lines -gt 1)
            }
            catch {
                return $false
            }
        }
        default { return $false }
    }
}

function Test-HfModelCache {
    $hfHome = if ([string]::IsNullOrWhiteSpace($env:HF_HOME)) { Join-Path $env:USERPROFILE ".cache\huggingface" } else { $env:HF_HOME }
    $hub = Join-Path $hfHome "hub"
    if (-not (Test-Path $hub)) { return $false }
    $entries = Get-ChildItem -Path $hub -Filter "models--*" -Directory -ErrorAction SilentlyContinue
    return ($entries.Count -gt 0)
}

function Test-ExistingLocalModelStack {
    foreach ($h in @("sglang","vllm","docker-model-runner","ollama")) {
        if (Test-HostReady $h) { return $true }
    }
    if (Test-HostHasModels "ollama") { return $true }
    if (Test-HfModelCache) { return $true }
    return $false
}

function Install-LocalHost {
    param([Parameter(Mandatory = $true)][string]$Host)
    switch ($Host) {
        "sglang" {
            if (-not (Test-Command "python")) {
                Write-Warn "Python not found. Cannot auto-install SGLang."
                return $false
            }
            & python -m pip install --user "sglang[all]"
            return ($LASTEXITCODE -eq 0)
        }
        "vllm" {
            if (-not (Test-Command "python")) {
                Write-Warn "Python not found. Cannot auto-install vLLM."
                return $false
            }
            & python -m pip install --user vllm
            return ($LASTEXITCODE -eq 0)
        }
        "docker-model-runner" {
            Write-Warn "Automatic Docker installation is not supported by this installer."
            return $false
        }
        "ollama" {
            Write-Warn "Automatic Ollama installation is not supported by this installer."
            return $false
        }
        default {
            return $false
        }
    }
}

function Install-ApertusForHost {
    param(
        [Parameter(Mandatory = $true)][string]$Host,
        [Parameter(Mandatory = $true)][ValidateSet("8b","70b")][string]$Variant
    )

    $repo = if ($Variant -eq "70b") { $Apertus70BRepo } else { $Apertus8BRepo }
    $ollamaModel = if ($Variant -eq "70b") { $Apertus70BOllama } else { $Apertus8BOllama }

    switch ($Host) {
        "ollama" {
            if (-not (Test-Command "ollama")) {
                Write-Warn "Ollama not found."
                return $false
            }
            & ollama pull $ollamaModel
            return ($LASTEXITCODE -eq 0)
        }
        "sglang" { }
        "vllm" { }
        "docker-model-runner" { }
        default {
            return $false
        }
    }

    if (-not (Test-Command "python")) {
        Write-Warn "Python not found. Cannot pre-download Apertus."
        return $false
    }

    $py = @"
from huggingface_hub import snapshot_download
snapshot_download(repo_id="$repo")
print("Downloaded $repo")
"@
    & python -c $py
    return ($LASTEXITCODE -eq 0)
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
Write-Info ("Cargo --locked: " + ($(if ($InstallLocked) { "enabled" } else { "disabled" })))

# ── Liability Waiver ──────────────────────────────────────────
Write-Host ""
Write-Step "Disclaimer"
Write-Info "IMPORTANT - PLEASE READ"
Write-Info ""
Write-Info "  Ironclad is an autonomous AI agent that can execute actions,"
Write-Info "  interact with external services, and manage digital assets"
Write-Info "  including cryptocurrency wallets and on-chain transactions."
Write-Info ""
Write-Info "  THE SOFTWARE IS PROVIDED `"AS IS`", WITHOUT WARRANTY OF ANY KIND."
Write-Info "  The developers and contributors bear no responsibility for:"
Write-Info ""
Write-Info "    - Actions taken by the agent, whether intended or unintended"
Write-Info "    - Loss of funds, income, cryptocurrency, or other digital assets"
Write-Info "    - Security vulnerabilities, compromises, or unauthorized access"
Write-Info "    - Damages arising from the agent's use, misuse, or malfunction"
Write-Info "    - Any financial, legal, or operational consequences whatsoever"
Write-Info ""
Write-Info "  By proceeding, you acknowledge that you use Ironclad entirely"
Write-Info "  at your own risk and accept full responsibility for its operation."
Write-Info ""

if (-not (Confirm-Step "I understand and accept these terms")) {
    Write-Warn "Installation cancelled by user."
    exit 0
}

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
if ($InstallLocked) {
    $installArgs += @("--locked")
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

Write-Step "Optional local Apertus setup"
Write-Info "Supported hosts: SGLang (recommended), vLLM, Docker Model Runner, Ollama."
if (Test-ExistingLocalModelStack) {
    Write-Info "Detected an existing local model framework or model cache."
    Write-Info "Skipping SGLang + Apertus onboarding recommendation."
    Write-Info "Use 'ironclad setup' if you want to switch to Apertus manually."
} elseif (Confirm-Step "Install a local Apertus model now?") {
    $ramGb = Get-TotalRamGb
    $vramGb = Get-GpuVramGb
    Write-Info "Detected RAM: $ramGb GB"
    if ($vramGb -gt 0) {
        Write-Info "Detected GPU VRAM: $vramGb GB"
    } else {
        Write-Info "GPU VRAM not detected (CPU-only assumptions will be used)"
    }

    $can8B = ($ramGb -ge 16) -or ($vramGb -ge 8)
    $can70B = ($ramGb -ge 64) -or ($vramGb -ge 40)
    if (-not $can8B -and -not $can70B) {
        Write-Warn "System resources are below recommended minimum for Apertus local runtime."
    } else {
        $hosts = @()
        foreach ($h in @("sglang","vllm","docker-model-runner","ollama")) {
            if (Test-HostReady $h) { $hosts += $h }
        }

        if ($hosts.Count -eq 0 -and (Confirm-Step "No host detected. Install/configure one now? (SGLang recommended)")) {
            if (Confirm-Step "Use SGLang (recommended)?") {
                if ((Install-LocalHost "sglang") -and (Test-HostReady "sglang")) { $hosts += "sglang" }
            } elseif (Confirm-Step "Use vLLM?" "N") {
                if ((Install-LocalHost "vllm") -and (Test-HostReady "vllm")) { $hosts += "vllm" }
            } elseif (Confirm-Step "Use Docker Model Runner?" "N") {
                if ((Install-LocalHost "docker-model-runner") -and (Test-HostReady "docker-model-runner")) { $hosts += "docker-model-runner" }
            } elseif (Confirm-Step "Use Ollama?" "N") {
                if ((Install-LocalHost "ollama") -and (Test-HostReady "ollama")) { $hosts += "ollama" }
            }
        }

        if ($hosts.Count -gt 0) {
            $selectedHost = $hosts[0]
            if ($hosts.Count -gt 1 -and -not (Confirm-Step "Use recommended host ($selectedHost)?")) {
                Write-Info "Available hosts: $($hosts -join ', ')"
                $entered = Read-Host "Enter host to use"
                if ($hosts -contains $entered) {
                    $selectedHost = $entered
                }
            }

            $variant = "8b"
            if ($can70B -and -not (Confirm-Step "Use Apertus 8B Instruct (recommended default)?")) {
                $variant = "70b"
            }

            Write-Info "Downloading Apertus ($variant) for $selectedHost..."
            if (Install-ApertusForHost -Host $selectedHost -Variant $variant) {
                Write-Info "Apertus download complete."
            } else {
                Write-Warn "Apertus download failed; continuing installation."
            }
        } else {
            Write-Warn "No host available after bootstrap attempt; skipping Apertus download."
        }
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
