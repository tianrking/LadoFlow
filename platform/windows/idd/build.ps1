[CmdletBinding()]
param(
    [ValidateSet('Debug', 'Release')]
    [string]$Configuration = 'Release',
    [switch]$NoRestore,
    [switch]$RequireSpectre,
    [string]$NuGetPath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$projectRoot = $PSScriptRoot
$solutionPath = Join-Path $projectRoot 'LadoFlowIdd.sln'
$packagesPath = Join-Path $projectRoot 'packages'
$toolsPath = Join-Path $projectRoot '.tools'
$wdkPackageName = 'Microsoft.Windows.WDK.x64.10.0.26100.6584'
$sdkPackageName = 'Microsoft.Windows.SDK.CPP.10.0.26100.1'
$sdkArchPackageName = 'Microsoft.Windows.SDK.CPP.x64.10.0.26100.1'

function Resolve-VisualStudio {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path -LiteralPath $vswhere)) {
        throw 'Visual Studio Installer (vswhere.exe) was not found. Install Visual Studio 2022 with Desktop development with C++.'
    }

    $installation = & $vswhere -latest -version '[17.0,18.0)' -products * `
        -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
        -property installationPath
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($installation)) {
        throw 'Visual Studio 2022 with the x64 C++ toolset was not found.'
    }

    $installation = $installation.Trim()
    $msbuild = Join-Path $installation 'MSBuild\Current\Bin\MSBuild.exe'
    $vcTargets = Join-Path $installation 'MSBuild\Microsoft\VC\v170'
    if (-not (Test-Path -LiteralPath $msbuild) -or -not (Test-Path -LiteralPath $vcTargets)) {
        throw "The Visual Studio C++ MSBuild files are incomplete at '$installation'."
    }

    return [PSCustomObject]@{
        Installation = $installation
        MsBuild = $msbuild
        VcTargets = $vcTargets
    }
}

function Resolve-NuGet {
    if (-not [string]::IsNullOrWhiteSpace($NuGetPath)) {
        if (-not (Test-Path -LiteralPath $NuGetPath)) {
            throw "NuGet was not found at '$NuGetPath'."
        }
        return (Resolve-Path -LiteralPath $NuGetPath).Path
    }

    $command = Get-Command nuget.exe -ErrorAction SilentlyContinue
    if ($null -ne $command) {
        return $command.Source
    }

    $localFallback = Join-Path $env:LOCALAPPDATA 'LadoFlow\tools\nuget\nuget.exe'
    if (Test-Path -LiteralPath $localFallback) {
        return $localFallback
    }

    throw 'nuget.exe is required. Install it with winget install Microsoft.NuGet, or pass -NuGetPath.'
}

function Restore-WdkPackages {
    $requiredProps = @(
        (Join-Path $packagesPath "$wdkPackageName\build\native\Microsoft.Windows.WDK.x64.props"),
        (Join-Path $packagesPath "$sdkPackageName\build\native\Microsoft.Windows.SDK.cpp.props"),
        (Join-Path $packagesPath "$sdkArchPackageName\build\native\Microsoft.Windows.SDK.cpp.x64.props")
    )
    $missing = @($requiredProps | Where-Object { -not (Test-Path -LiteralPath $_) })
    if ($missing.Count -eq 0) {
        return
    }
    if ($NoRestore) {
        throw 'Pinned WDK packages are missing and -NoRestore was supplied.'
    }

    $nuget = Resolve-NuGet
    & $nuget restore (Join-Path $projectRoot 'packages.config') `
        -PackagesDirectory $packagesPath `
        -Source 'https://api.nuget.org/v3/index.json' `
        -NonInteractive
    if ($LASTEXITCODE -ne 0) {
        throw "NuGet restore failed with exit code $LASTEXITCODE."
    }

    $stillMissing = @($requiredProps | Where-Object { -not (Test-Path -LiteralPath $_) })
    if ($stillMissing.Count -ne 0) {
        throw "NuGet restore completed but required WDK files are missing: $($stillMissing -join ', ')"
    }
}

function Resolve-WdkVcTargets([string]$baseTargets) {
    $toolsetRelative = 'Platforms\x64\PlatformToolsets\WindowsUserModeDriver10.0\Toolset.props'
    if (Test-Path -LiteralPath (Join-Path $baseTargets $toolsetRelative)) {
        return $baseTargets
    }

    # Microsoft ships the small Visual Studio WDK integration layer separately
    # from the NuGet WDK payload. Keep a user-local overlay so builds need no
    # administrative write to Program Files.
    $overlay = Join-Path $toolsPath 'msbuild-v170-wdk'
    $overlayToolset = Join-Path $overlay $toolsetRelative
    if (Test-Path -LiteralPath $overlayToolset) {
        return $overlay
    }

    $payloadUri = 'https://download.visualstudio.microsoft.com/download/pr/fa1259b6-3659-4a26-a8b4-c42d40b343ab/c53677dd5d56679c4298323fd12a1ec504cc05e9e346c0866b178955a9dcbf4b/payload.vsix'
    $expectedSha256 = 'c53677dd5d56679c4298323fd12a1ec504cc05e9e346c0866b178955a9dcbf4b'
    $cache = Join-Path $toolsPath 'cache'
    $vsix = Join-Path $cache 'Microsoft.VisualStudio.WindowsDriverKit.Build-17.14.36705.7.vsix'
    $expanded = Join-Path $toolsPath 'wdk-build-vsix-17.14.36705.7'
    New-Item -ItemType Directory -Force -Path $cache, $overlay | Out-Null

    $downloadRequired = -not (Test-Path -LiteralPath $vsix)
    if (-not $downloadRequired) {
        $downloadRequired = (Get-FileHash -Algorithm SHA256 -LiteralPath $vsix).Hash.ToLowerInvariant() -ne $expectedSha256
    }
    if ($downloadRequired) {
        Invoke-WebRequest -Uri $payloadUri -OutFile $vsix
    }
    $actualHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $vsix).Hash.ToLowerInvariant()
    if ($actualHash -ne $expectedSha256) {
        throw "WDK Visual Studio integration payload hash mismatch: $actualHash"
    }

    if (-not (Test-Path -LiteralPath (Join-Path $expanded 'manifest.json'))) {
        if (Test-Path -LiteralPath $expanded) {
            throw "Incomplete WDK integration cache at '$expanded'. Remove only that directory and retry."
        }
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        [System.IO.Compression.ZipFile]::ExtractToDirectory($vsix, $expanded)
    }

    Copy-Item -Path (Join-Path $baseTargets '*') -Destination $overlay -Recurse -Force
    $integrationRoot = Join-Path $expanded 'Contents\MSBuild\Microsoft\VC\v170'
    Copy-Item -Path (Join-Path $integrationRoot '*') -Destination $overlay -Recurse -Force
    if (-not (Test-Path -LiteralPath $overlayToolset)) {
        throw 'The local WDK MSBuild overlay did not contain the expected driver toolset.'
    }
    return $overlay
}

function Resolve-SpectreSetting([string]$visualStudioPath) {
    $msvcRoot = Join-Path $visualStudioPath 'VC\Tools\MSVC'
    $spectreLibrary = Get-ChildItem -LiteralPath $msvcRoot -Directory -ErrorAction SilentlyContinue |
        Sort-Object Name -Descending |
        ForEach-Object { Join-Path $_.FullName 'lib\spectre\x64\vcruntime.lib' } |
        Where-Object { Test-Path -LiteralPath $_ } |
        Select-Object -First 1
    if ($null -ne $spectreLibrary) {
        return 'Spectre'
    }
    if ($RequireSpectre) {
        throw 'The x64 Spectre-mitigated MSVC libraries are required but not installed.'
    }
    Write-Warning 'Spectre-mitigated MSVC libraries are absent; producing a development-only driver build.'
    return 'false'
}

Restore-WdkPackages
$visualStudio = Resolve-VisualStudio
$vcTargets = Resolve-WdkVcTargets $visualStudio.VcTargets
$spectreSetting = Resolve-SpectreSetting $visualStudio.Installation

$wdkRoot = Join-Path $packagesPath "$wdkPackageName\c"
$wdkBin = Join-Path $wdkRoot 'bin\10.0.26100.0'
$msbuildArguments = @(
    $solutionPath,
    '/m:1',
    '/nr:false',
    '/t:Rebuild',
    "/p:Configuration=$Configuration",
    '/p:Platform=x64',
    '/p:Processor_Architecture=AMD64',
    "/p:Driver_SpectreMitigation=$spectreSetting",
    "/p:SpectreMitigation=$spectreSetting",
    '/p:LadoFlowEnablePrefast=false',
    '/p:TrackFileAccess=false',
    '/p:SkipPackageVerification=true',
    '/p:ApiValidator_Enable=true',
    '/p:Inf2CatUseLocalTime=true',
    "/p:InfToolPath=$wdkBin\x64\",
    "/p:Inf2CatToolPath=$wdkBin\x86\",
    "/p:DrvCatToolPath=$wdkBin\x64\",
    "/p:VCTargetsPath=$vcTargets\",
    "/bl:$projectRoot\ladoflow-idd-$($Configuration.ToLowerInvariant()).binlog",
    '/v:minimal'
)

& $visualStudio.MsBuild @msbuildArguments
if ($LASTEXITCODE -ne 0) {
    throw "MSBuild failed with exit code $LASTEXITCODE."
}

$outputRoot = Join-Path $projectRoot "x64\$Configuration"
$generatedInf = Join-Path $outputRoot 'LadoFlowIdd.inf'
$infVerifier = Join-Path $wdkRoot 'tools\10.0.26100.0\x64\infverif.exe'
& $infVerifier /u /v $generatedInf
if ($LASTEXITCODE -ne 0) {
    throw "InfVerif failed with exit code $LASTEXITCODE."
}

$controller = Join-Path $outputRoot 'LadoFlowVirtualDisplay.exe'
& $controller self-test
if ($LASTEXITCODE -ne 0) {
    throw "The lifecycle controller self-test failed with exit code $LASTEXITCODE."
}
& $controller status
if ($LASTEXITCODE -ne 0) {
    throw "The lifecycle controller status smoke test failed with exit code $LASTEXITCODE."
}

$service = Join-Path $outputRoot 'LadoFlowDisplayService.exe'
& $service self-test
if ($LASTEXITCODE -ne 0) {
    throw "The privileged service self-test failed with exit code $LASTEXITCODE."
}

$setup = Join-Path $outputRoot 'LadoFlowWindowsSetup.exe'
& $setup self-test
if ($LASTEXITCODE -ne 0) {
    throw "The Windows setup helper self-test failed with exit code $LASTEXITCODE."
}

$artifactRoot = Join-Path $projectRoot "dist\$Configuration\x64"
$driverArtifact = Join-Path $artifactRoot 'driver'
$symbolsArtifact = Join-Path $artifactRoot 'symbols'
New-Item -ItemType Directory -Force -Path $driverArtifact, $symbolsArtifact | Out-Null
Copy-Item -Path (Join-Path $outputRoot 'LadoFlowIdd\*') -Destination $driverArtifact -Recurse -Force
Copy-Item -LiteralPath $controller -Destination $artifactRoot -Force
Copy-Item -LiteralPath $service -Destination $artifactRoot -Force
Copy-Item -LiteralPath $setup -Destination $artifactRoot -Force

$packagedSetup = Join-Path $artifactRoot 'LadoFlowWindowsSetup.exe'
& $packagedSetup plan-install
if ($LASTEXITCODE -ne 0) {
    throw "The packaged Windows setup plan failed with exit code $LASTEXITCODE."
}

$certificate = Join-Path $outputRoot 'LadoFlowIdd.cer'
if (Test-Path -LiteralPath $certificate) {
    Copy-Item -LiteralPath $certificate -Destination $artifactRoot -Force
}
Get-ChildItem -LiteralPath $outputRoot -Filter '*.pdb' | ForEach-Object {
    Copy-Item -LiteralPath $_.FullName -Destination $symbolsArtifact -Force
}

Write-Host "LadoFlow Windows IDD build passed. Artifacts: $artifactRoot"
