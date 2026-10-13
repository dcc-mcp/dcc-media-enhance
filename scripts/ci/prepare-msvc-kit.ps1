# Prepare a job-local view of installed Microsoft tools for the project's own
# msvc-kit. No SDK install, registry edit, machine PATH change, or doctor substitute.
[CmdletBinding()]
param(
    [string] $MsvcVersion = $env:DCC_MEDIA_MSVC_VERSION,
    [string] $SdkVersion = $env:DCC_MEDIA_SDK_VERSION,
    [string] $HostArch = $env:DCC_MEDIA_HOST_ARCH
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not $IsWindows -or $PSVersionTable.PSVersion.Major -lt 7) {
    throw 'prepare-msvc-kit.ps1 requires PowerShell 7 on an existing Windows runner.'
}
if (-not $env:RUNNER_TEMP -or -not $env:GITHUB_ENV) {
    throw 'RUNNER_TEMP and GITHUB_ENV are required; this preparation is scoped to one CI job.'
}
if (-not $HostArch) { $HostArch = 'x64' }
if ($HostArch -cnotin @('x64', 'x86')) {
    throw 'DCC_MEDIA_HOST_ARCH must be x64 or x86 for the x64 target.'
}
if ($MsvcVersion -eq '14.0') {
    throw 'VS2015 requires an explicit original installation root and SDK root; this CI preparation only discovers modern VS.'
}
if ($MsvcVersion -and $MsvcVersion -notmatch '^\d+\.\d+\.\d+$') {
    throw 'DCC_MEDIA_MSVC_VERSION must be a full version, for example 14.44.35207.'
}
if ($SdkVersion -and $SdkVersion -notmatch '^\d+\.\d+\.\d+\.\d+$') {
    throw 'DCC_MEDIA_SDK_VERSION must be a full version, for example 10.0.26100.0.'
}

function Test-RequiredFiles {
    param([string] $Root, [string[]] $RelativePaths)
    foreach ($relative in $RelativePaths) {
        if (-not (Test-Path -LiteralPath (Join-Path $Root $relative) -PathType Leaf)) {
            return $false
        }
    }
    return $true
}

function Get-VisualStudioRoots {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    if (-not (Test-Path -LiteralPath $vswhere -PathType Leaf)) {
        throw 'Registered Visual Studio discovery is unavailable. Provision the runner C++ workload before this job.'
    }
    $start = [System.Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $vswhere
    $start.Arguments = '-all -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath -utf8'
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.StandardOutputEncoding = [System.Text.Encoding]::UTF8
    $process = [System.Diagnostics.Process]::Start($start)
    try {
        $output = $process.StandardOutput.ReadToEndAsync()
        if (-not $process.WaitForExit(15000)) {
            $process.Kill()
            throw 'Registered Visual Studio discovery timed out; no toolchain was selected.'
        }
        if ($process.ExitCode -ne 0) {
            throw "Registered Visual Studio discovery failed with exit code $($process.ExitCode)."
        }
        return @($output.Result -split '\r?\n' | ForEach-Object { $_.Trim() } | Where-Object { $_ } | Sort-Object -Unique)
    }
    finally {
        $process.Dispose()
    }
}

function Get-WindowsSdkRoots {
    foreach ($view in @([Microsoft.Win32.RegistryView]::Registry32, [Microsoft.Win32.RegistryView]::Registry64)) {
        $registry = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::LocalMachine, $view)
        try {
            $key = $registry.OpenSubKey('SOFTWARE\Microsoft\Windows Kits\Installed Roots')
            if ($null -eq $key) { continue }
            try {
                $root = $key.GetValue('KitsRoot10')
                if ($root -is [string] -and $root.Trim()) { $root.Trim() }
            }
            finally { $key.Dispose() }
        }
        finally { $registry.Dispose() }
    }
}

$compilers = @(
    foreach ($vsRoot in @(Get-VisualStudioRoots)) {
        $toolsRoot = Join-Path $vsRoot 'VC/Tools/MSVC'
        if (-not (Test-Path -LiteralPath $toolsRoot -PathType Container)) { continue }
        foreach ($directory in Get-ChildItem -LiteralPath $toolsRoot -Directory) {
            if ($directory.Name -notmatch '^\d+\.\d+\.\d+$') { continue }
            if ($MsvcVersion -and $directory.Name -ne $MsvcVersion) { continue }
            if (Test-RequiredFiles $directory.FullName @(
                "bin/Host$HostArch/x64/cl.exe", "bin/Host$HostArch/x64/link.exe", "bin/Host$HostArch/x64/lib.exe",
                'include/vcruntime.h', 'include/vector', 'lib/x64/msvcrt.lib', 'lib/x64/libcmt.lib',
                'lib/x64/vcruntime.lib', 'lib/x64/msvcprt.lib'
            )) {
                [pscustomobject]@{
                    Version = [version] $directory.Name
                    VersionText = $directory.Name
                    Root = $directory.FullName
                    VisualStudioRoot = $vsRoot
                    ToolsRoot = $toolsRoot
                }
            }
        }
    }
)
$selectedCompiler = $compilers | Sort-Object -Property @{ Expression = 'Version'; Descending = $true }, VisualStudioRoot | Select-Object -First 1
if ($null -eq $selectedCompiler) {
    throw "No complete registered x64 MSVC toolset matches '$MsvcVersion'. Provision it explicitly; this script does not install toolchains."
}

$sdks = @(
    foreach ($sdkRoot in @(Get-WindowsSdkRoots | Sort-Object -Unique)) {
        $includeRoot = Join-Path $sdkRoot 'Include'
        if (-not (Test-Path -LiteralPath $includeRoot -PathType Container)) { continue }
        foreach ($directory in Get-ChildItem -LiteralPath $includeRoot -Directory) {
            if ($directory.Name -notmatch '^10\.0\.\d+\.\d+$') { continue }
            if ($SdkVersion -and $directory.Name -ne $SdkVersion) { continue }
            $version = $directory.Name
            if (Test-RequiredFiles $sdkRoot @(
                "Include/$version/um/Windows.h", "Include/$version/ucrt/stdio.h",
                "Include/$version/shared/sdkddkver.h", "Lib/$version/um/x64/kernel32.lib",
                "Lib/$version/ucrt/x64/ucrt.lib", "bin/$version/$HostArch/rc.exe", "bin/$version/$HostArch/mt.exe"
            )) {
                [pscustomobject]@{ Version = [version] $version; VersionText = $version; Root = $sdkRoot }
            }
        }
    }
)
$selectedSdk = $sdks | Sort-Object -Property @{ Expression = 'Version'; Descending = $true }, Root | Select-Object -First 1
if ($null -eq $selectedSdk) {
    throw "No complete registered Windows SDK matches '$SdkVersion'. Provision its x64 headers, libraries, rc.exe and mt.exe before this job."
}

# Never reuse or delete a user directory. Junctions are read-only inputs by usage:
# doctor writes its native probe into its own temporary directory, not these roots.
# Do not recursively remove a junction or its registered VS/SDK target.
$workspace = Join-Path $env:RUNNER_TEMP ('dcc-mcp-msvc-kit-' + [guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $workspace
$cli = Join-Path $workspace 'msvc-kit.exe'
$downloadUrl = 'https://github.com/loonghao/msvc-kit/releases/download/v0.2.22/msvc-kit-x86_64-windows.exe'
$expectedSha = '184f07fb2896f0a7d27c45445990bd7d9e525b6ee58956cfe857bf0027848925'
Invoke-WebRequest -Uri $downloadUrl -OutFile $cli -TimeoutSec 120
$actualSha = (Get-FileHash -LiteralPath $cli -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualSha -ne $expectedSha) {
    throw "msvc-kit v0.2.22 integrity mismatch: expected $expectedSha, got $actualSha. The binary was not executed."
}

$layout = Join-Path $workspace 'toolchain'
$null = New-Item -ItemType Directory -Path $layout
$null = New-Item -ItemType Directory -Path (Join-Path $layout 'VC')
$null = New-Item -ItemType Directory -Path (Join-Path $layout 'VC/Tools')
$null = New-Item -ItemType Directory -Path (Join-Path $layout 'Windows Kits')
$vcLink = Join-Path $layout 'VC/Tools/MSVC'
$sdkLink = Join-Path $layout 'Windows Kits/10'
$null = New-Item -ItemType Junction -Path $vcLink -Value $selectedCompiler.ToolsRoot
$null = New-Item -ItemType Junction -Path $sdkLink -Value $selectedSdk.Root

$tools = [ordered]@{}
foreach ($name in @('cl', 'link', 'lib')) {
    $path = Join-Path $selectedCompiler.Root "bin/Host$HostArch/x64/$name.exe"
    $tools[$name] = [ordered]@{ registered_path = $path; sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() }
}
foreach ($name in @('rc', 'mt')) {
    $path = Join-Path $selectedSdk.Root "bin/$($selectedSdk.VersionText)/$HostArch/$name.exe"
    $tools[$name] = [ordered]@{ registered_path = $path; sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() }
}
$manifest = Join-Path $workspace 'preparation.json'
[ordered]@{
    schema = 'dcc-media-enhance.msvc-kit-preparation.v1'
    cli_release = 'v0.2.22'
    cli_url = $downloadUrl
    cli_path = $cli
    cli_sha256 = $actualSha
    host_arch = $HostArch
    target_arch = 'x64'
    selected_msvc_version = $selectedCompiler.VersionText
    selected_sdk_version = $selectedSdk.VersionText
    registered_visual_studio_root = $selectedCompiler.VisualStudioRoot
    registered_sdk_root = $selectedSdk.Root
    toolchain_view = $layout
    junctions = @(
        @{ path = $vcLink; target = $selectedCompiler.ToolsRoot },
        @{ path = $sdkLink; target = $selectedSdk.Root }
    )
    tools = $tools
    doctor_executed = $false
    scope = 'Preparation only. Consumer tests/builds must run the real doctor --compile and retain its report.'
} | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $manifest -Encoding utf8

$exports = [ordered]@{
    DCC_MEDIA_MSVC_KIT = $cli
    DCC_MEDIA_MSVC_KIT_DIR = $layout
    DCC_MEDIA_MSVC_VERSION = $selectedCompiler.VersionText
    DCC_MEDIA_SDK_VERSION = $selectedSdk.VersionText
    DCC_MEDIA_HOST_ARCH = $HostArch
    DCC_MEDIA_MSVC_KIT_PREPARATION = $manifest
}
foreach ($pair in $exports.GetEnumerator()) {
    if ($pair.Value -match '[\r\n]') { throw 'Multiline toolchain values cannot be exported to the CI environment.' }
    Add-Content -LiteralPath $env:GITHUB_ENV -Value "$($pair.Key)=$($pair.Value)" -Encoding utf8
}
Write-Host "Prepared msvc-kit v0.2.22 with registered MSVC $($selectedCompiler.VersionText) and SDK $($selectedSdk.VersionText), $HostArch host / x64 target."
Write-Host "Preparation evidence: $manifest. No doctor or Rust build has been claimed by this step."
