$ErrorActionPreference = 'Stop'
# Agent distributions must not silently depend on a preinstalled VC runtime.
$env:RUSTFLAGS = '-C target-feature=+crt-static'
$selection = @('--dir', $env:DCC_MEDIA_MSVC_KIT_DIR, '--arch', 'x64', '--host-arch', 'x64', '--msvc-version', $env:DCC_MEDIA_MSVC_VERSION, '--sdk-version', $env:DCC_MEDIA_SDK_VERSION)
$report = & $env:DCC_MEDIA_MSVC_KIT doctor @selection --compile --format json
if ($LASTEXITCODE -ne 0) { throw 'Pinned msvc-kit doctor failed; no fallback compiler is permitted.' }
$report | Set-Content -Encoding utf8 msvc-kit-doctor.json
$data = $report | ConvertFrom-Json
if ($data.schema -ne 'msvc-kit.doctor.v1' -or $data.status -ne 'passed') { throw 'Unexpected doctor result.' }
foreach ($stage in @('compile', 'resource', 'link', 'manifest', 'execute')) {
    $check = @($data.checks | Where-Object { $_.id -eq "probe.$stage" })
    if ($check.Count -ne 1 -or $check[0].status -ne 'passed') { throw "Doctor did not pass probe.$stage" }
}
if ($data.toolchain.arch -ne 'x64' -or $data.toolchain.host_arch -ne 'x64') { throw 'Wrong doctor architecture.' }
# Every Cargo compiler/linker invocation inherits the exact kit-selected tools.
& $env:DCC_MEDIA_MSVC_KIT run @selection -- cargo test --locked --release
if ($LASTEXITCODE -ne 0) { throw 'Native Windows unit tests failed.' }
& $env:DCC_MEDIA_MSVC_KIT run @selection -- cargo build --locked --release
if ($LASTEXITCODE -ne 0) { throw 'Native Windows release build failed.' }
