param([switch]$Release, [string]$OutputDirectory)
$ErrorActionPreference = "Stop"

$projectRoot = Split-Path -Parent $PSScriptRoot
$androidProject = Join-Path $projectRoot "android-app"
$variant = if ($Release) { "release" } else { "debug" }
$version = (Get-Content (Join-Path $projectRoot "package.json") -Raw -Encoding UTF8 | ConvertFrom-Json).version
$sourceApk = Join-Path $androidProject "app\build\outputs\apk\$variant\app-$variant.apk"
if ($Release) {
    foreach ($name in @('PLUMELINK_KEYSTORE', 'PLUMELINK_STORE_PASSWORD', 'PLUMELINK_KEY_ALIAS', 'PLUMELINK_KEY_PASSWORD')) {
        if (-not [Environment]::GetEnvironmentVariable($name)) { throw "Missing release signing configuration: $name" }
    }
    if (-not (Test-Path -LiteralPath $env:PLUMELINK_KEYSTORE -PathType Leaf)) { throw "Release keystore file was not found." }
}

$javaHome = $env:JAVA_HOME
if (-not $javaHome) {
    $javaHome = Get-ChildItem "C:\Program Files\Microsoft" -Directory -Filter "jdk-17*-hotspot" -ErrorAction SilentlyContinue |
        Sort-Object Name -Descending |
        Select-Object -First 1 -ExpandProperty FullName
}
if (-not $javaHome -or -not (Test-Path -LiteralPath (Join-Path $javaHome "bin\java.exe"))) {
    throw "JDK 17 was not found. Install Microsoft.OpenJDK.17 or set JAVA_HOME."
}

$androidHome = $env:ANDROID_HOME
if (-not $androidHome) {
    $androidHome = Join-Path $env:LOCALAPPDATA "Android\Sdk"
}
if (-not (Test-Path -LiteralPath (Join-Path $androidHome "platforms\android-35"))) {
    throw "Android SDK Platform 35 was not found. Set ANDROID_HOME and install platforms;android-35."
}

$env:JAVA_HOME = $javaHome
$env:ANDROID_HOME = $androidHome

Push-Location $projectRoot
try {
    & node "scripts/build-frontend.mjs"
    if ($LASTEXITCODE -ne 0) { throw "Mobile UI build failed with exit code $LASTEXITCODE." }
} finally {
    Pop-Location
}

Push-Location $androidProject
try {
    if ($Release) { & ".\gradlew.bat" testDebugUnitTest assembleRelease lintRelease } else { & ".\gradlew.bat" testDebugUnitTest assembleDebug lintDebug }
    if ($LASTEXITCODE -ne 0) { throw "Gradle build failed with exit code $LASTEXITCODE." }
} finally {
    Pop-Location
}

if (-not (Test-Path -LiteralPath $sourceApk)) {
    throw "Gradle completed but the APK was not found: $sourceApk"
}
if ($Release) {
    & (Join-Path $androidHome "build-tools/35.0.0/apksigner.bat") verify --verbose --print-certs $sourceApk
    if ($LASTEXITCODE -ne 0) { throw "Release APK signature verification failed." }
    & (Join-Path $PSScriptRoot "verify-build-paths.ps1") -Files $sourceApk
}
$releaseApk = $sourceApk
if ($OutputDirectory) {
    New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
    $releaseApk = Join-Path $OutputDirectory "PlumeLink-Android-$version-$variant.apk"
    Copy-Item -LiteralPath $sourceApk -Destination $releaseApk -Force
}
$stream = [System.IO.File]::OpenRead($releaseApk)
try {
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        $hash = ([System.BitConverter]::ToString($sha.ComputeHash($stream))).Replace("-", "")
    } finally {
        $sha.Dispose()
    }
} finally {
    $stream.Dispose()
}
Write-Host "Android APK: $releaseApk"
Write-Host "SHA-256: $hash"
