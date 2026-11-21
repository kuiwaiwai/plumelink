$ErrorActionPreference = 'Stop'
$archive = Join-Path (Get-Location) 'VB-CABLE.zip'
$bundled = $env:PLUMELINK_VBCABLE_ARCHIVE
if ($bundled) {
    try { Copy-Item -LiteralPath $bundled -Destination $archive -ErrorAction Stop }
    catch { exit 10 }
} else {
    $url = 'https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack46.zip'
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    try { Invoke-WebRequest -UseBasicParsing -TimeoutSec 90 -Uri $url -OutFile $archive }
    catch { exit 10 }
}
if ((Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash -ne 'BC44E8974191A5A2427D6C979E2DAB08DBB09925F3F435DFA2F0BA3E45B6EB90') { exit 11 }
$files = Join-Path (Get-Location) 'setup'
Expand-Archive -LiteralPath $archive -DestinationPath $files
$setup = Join-Path $files 'VBCABLE_Setup_x64.exe'
if ((Get-AuthenticodeSignature -LiteralPath $setup).Status -ne 'Valid') { exit 11 }
try {
    $process = Start-Process -FilePath $setup -WorkingDirectory $files -Verb RunAs -PassThru -Wait
} catch {
    $cause = $_.Exception
    while ($cause) {
        if ($cause -is [ComponentModel.Win32Exception] -and $cause.NativeErrorCode -eq 1223) { exit 12 }
        $cause = $cause.InnerException
    }
    exit 13
}
if ($process.ExitCode -notin @(0, 3010)) { exit 14 }
