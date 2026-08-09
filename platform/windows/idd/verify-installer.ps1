[CmdletBinding()]
param(
    [string]$TauriTargetRoot
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ([string]::IsNullOrWhiteSpace($TauriTargetRoot)) {
    $repositoryRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..\..')).Path
    $TauriTargetRoot = Join-Path $repositoryRoot 'target'
}

$targetRoot = (Resolve-Path -LiteralPath $TauriTargetRoot).Path
$installerScript = Join-Path $targetRoot 'release\nsis\x64\installer.nsi'
$hookScript = Join-Path $PSScriptRoot '..\..\..\apps\desktop\src-tauri\windows\nsis-hooks.nsh'
if (-not (Test-Path -LiteralPath $installerScript -PathType Leaf)) {
    throw "Generated NSIS script was not found at '$installerScript'."
}
if (-not (Test-Path -LiteralPath $hookScript -PathType Leaf)) {
    throw "LadoFlow NSIS hooks were not found at '$hookScript'."
}

function Assert-Contains {
    param(
        [Parameter(Mandatory)]
        [string]$Content,
        [Parameter(Mandatory)]
        [string]$Expected,
        [Parameter(Mandatory)]
        [string]$Label
    )

    if ($Content.IndexOf($Expected, [StringComparison]::Ordinal) -lt 0) {
        throw "Generated installer verification failed: $Label."
    }
}

$generated = Get-Content -LiteralPath $installerScript -Raw
$hooks = Get-Content -LiteralPath $hookScript -Raw

Assert-Contains $generated '!define INSTALLMODE "perMachine"' 'per-machine install mode is missing'
Assert-Contains $generated 'RequestExecutionLevel admin' 'administrator execution level is missing'
Assert-Contains $generated '!insertmacro NSIS_HOOK_PREINSTALL' 'pre-install hook is not invoked'
Assert-Contains $generated '!insertmacro NSIS_HOOK_POSTINSTALL' 'post-install hook is not invoked'
Assert-Contains $generated '!insertmacro NSIS_HOOK_PREUNINSTALL' 'pre-uninstall hook is not invoked'
Assert-Contains $generated 'windows\LadoFlowWindowsSetup.exe' 'native setup helper is not bundled'
Assert-Contains $generated 'windows\LadoFlowDisplayService.exe' 'LocalSystem service is not bundled'
Assert-Contains $generated 'windows\LadoFlowVirtualDisplay.exe' 'unprivileged controller is not bundled'
Assert-Contains $generated 'windows\driver\LadoFlowIdd.inf' 'driver INF is not bundled'
Assert-Contains $generated 'windows\driver\ladoflowidd.cat' 'driver catalog is not bundled'
Assert-Contains $hooks ' prepare-install' 'upgrade preparation command is missing'
Assert-Contains $hooks ' install' 'driver/service install command is missing'
Assert-Contains $hooks ' uninstall' 'driver/service uninstall command is missing'
Assert-Contains $hooks '$UpdateMode == 1' 'update-safe uninstall branch is missing'
Assert-Contains $hooks 'CheckIfAppIsRunning "ladoflow-desktop.exe" "LadoFlow"' 'pre-mutation process check is missing'

$installers = @(Get-ChildItem -LiteralPath (Join-Path $targetRoot 'release\bundle\nsis') -Filter '*.exe' -File)
if ($installers.Count -ne 1) {
    throw "Expected exactly one NSIS installer, found $($installers.Count)."
}

$hash = Get-FileHash -Algorithm SHA256 -LiteralPath $installers[0].FullName
[PSCustomObject]@{
    ok = $true
    installMode = 'perMachine'
    installer = $installers[0].FullName
    bytes = $installers[0].Length
    sha256 = $hash.Hash
    hooks = @('prepare-install', 'install', 'uninstall')
} | ConvertTo-Json -Compress
