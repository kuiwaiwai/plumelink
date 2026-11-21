$ErrorActionPreference = 'Stop'
$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$release = Join-Path $root 'release'
$artifacts = Join-Path $root 'artifacts'
$version = (Get-Content (Join-Path $root 'package.json') -Raw -Encoding UTF8 | ConvertFrom-Json).version
if ($version -notmatch '^\d+\.\d+\.\d+([-.][0-9A-Za-z.-]+)?$') { throw 'Invalid package version.' }
$config = Get-Content (Join-Path $root 'src-tauri/tauri.conf.json') -Raw -Encoding UTF8 | ConvertFrom-Json
if ($config.version -ne $version) { throw 'Windows and package versions differ.' }
$prefix = "PlumeLink-Windows-$version-x64"
if ($config.mainBinaryName -ne 'PlumeLink') { throw 'Unexpected installed executable name.' }
$exe = Join-Path $root "src-tauri/target/release/$($config.mainBinaryName).exe"
$nsis = Join-Path $root "src-tauri/target/release/bundle/nsis/$($config.productName)_${version}_x64-setup.exe"
$driver = Join-Path $artifacts 'VBCABLE_Driver_Pack46.zip'
$driverHash = 'bc44e8974191a5a2427d6c979e2dab08dbb09925f3f435dfa2f0ba3e45b6eb90'

function Run-Tauri([string[]]$Arguments) {
    & node scripts/tauri.mjs @Arguments
    if ($LASTEXITCODE -ne 0) { throw "Tauri $($Arguments[0]) failed with exit code $LASTEXITCODE." }
}

function Get-Sha256([string]$Path) {
    $stream = [IO.File]::OpenRead($Path)
    $sha = [Security.Cryptography.SHA256]::Create()
    try { return [BitConverter]::ToString($sha.ComputeHash($stream)).Replace('-', '') }
    finally { $sha.Dispose(); $stream.Dispose() }
}

function Assert-LocalDirectory([string]$Path, [string]$Parent) {
    if ([IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($Path)) -ne $Parent) {
        throw "Unexpected packaging path: $Path"
    }
    if (Test-Path -LiteralPath $Path) {
        $item = Get-Item -LiteralPath $Path -Force
        if (!$item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw "Expected a regular directory: $Path"
        }
    }
}

# Gradle and NSIS use shared build outputs. A process exit releases this mutex.
$mutex = New-Object System.Threading.Mutex($false, 'Local\PlumeLinkReleasePack')
$held = $false
$work = $null
Push-Location $root
try {
    try { $held = $mutex.WaitOne(0) } catch [Threading.AbandonedMutexException] { $held = $true }
    if (!$held) { throw 'Another PlumeLink packaging job is running.' }
    Assert-LocalDirectory $artifacts $root
    Assert-LocalDirectory $release $root
    New-Item -ItemType Directory -Force -Path $artifacts | Out-Null
    $id = [guid]::NewGuid().ToString('N')
    $work = Join-Path $artifacts "pack-$id"
    $ready = Join-Path $work 'ready'
    New-Item -ItemType Directory -Path $ready -Force | Out-Null

    & (Join-Path $PSScriptRoot 'build-android.ps1') -Release -OutputDirectory $ready
    Run-Tauri -Arguments @('build', '--no-bundle')
    & (Join-Path $PSScriptRoot 'verify-build-paths.ps1') -Files $exe

    if (!(Test-Path -LiteralPath $driver) -or (Get-Sha256 $driver) -ne $driverHash) {
        $download = Join-Path $work 'driver.zip'
        & curl.exe --fail --location --silent --show-error --max-time 90 --output $download 'https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack46.zip'
        if ($LASTEXITCODE -ne 0 -or (Get-Sha256 $download) -ne $driverHash) { throw 'VB-CABLE download or hash verification failed.' }
        Copy-Item -LiteralPath $download -Destination $driver -Force
    }
    $overlay = Join-Path $work 'driver-config.json'
    [IO.File]::WriteAllText($overlay, '{"bundle":{"resources":{"../artifacts/VBCABLE_Driver_Pack46.zip":"VBCABLE_Driver_Pack46.zip"}}}')
    Run-Tauri -Arguments @('bundle', '--bundles', 'nsis', '--config', $overlay)
    Copy-Item -LiteralPath $nsis -Destination (Join-Path $ready "$prefix-setup-with-vb-cable.exe")

    $files = @(Get-ChildItem -LiteralPath $ready -File)
    $expected = @("PlumeLink-Android-$version-release.apk", "$prefix-setup-with-vb-cable.exe")
    if ($files.Count -ne $expected.Count -or (Compare-Object $expected $files.Name) -or ($files | Where-Object Length -eq 0)) { throw 'Incomplete release set.' }
    foreach ($file in $files) {
        $hash = Get-Sha256 $file.FullName
        Write-Host "$hash  $($file.Name)"
    }

    # Publish only after every build succeeds. Preserve unrelated files from an older output folder.
    $previous = Join-Path $artifacts "previous-release-$id"
    Assert-LocalDirectory $ready $work
    Assert-LocalDirectory $previous $artifacts
    if (Test-Path -LiteralPath $release) { [IO.Directory]::Move($release, $previous) }
    try { [IO.Directory]::Move($ready, $release) }
    catch {
        if (Test-Path -LiteralPath $previous) { [IO.Directory]::Move($previous, $release) }
        throw
    }
    if (Test-Path -LiteralPath $previous) {
        foreach ($file in $files) {
            $old = Join-Path $previous $file.Name
            if (Test-Path -LiteralPath $old -PathType Leaf) { Remove-Item -LiteralPath $old }
        }
        if (@(Get-ChildItem -LiteralPath $previous -Force).Count -eq 0) { [IO.Directory]::Delete($previous) }
        else { Write-Host "Other previous files preserved in $previous" }
    }
    Write-Host "Release ready: $release"
} finally {
    try {
        if ($work -and (Test-Path -LiteralPath $work)) {
            Assert-LocalDirectory $work $artifacts
            Remove-Item -LiteralPath $work -Recurse -Force
        }
    } finally {
        Pop-Location
        if ($held) { $mutex.ReleaseMutex() }
        $mutex.Dispose()
    }
}
