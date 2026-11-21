param([Parameter(Mandatory, Position = 0)][string[]]$Files)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression.FileSystem

$root = (Get-Item -LiteralPath (Join-Path $PSScriptRoot '..')).FullName
$roots = @($root, $env:USERPROFILE, $env:CARGO_HOME, $env:RUSTUP_HOME) | Where-Object { $_ }
$needles = foreach ($item in $roots) {
    $item
    $item.Replace('\', '/')
    $item.Replace('\', '\\')
}
$encoding = [Text.Encoding]::GetEncoding(28591)
$patterns = foreach ($needle in ($needles | Select-Object -Unique)) {
    $encoding.GetString([Text.Encoding]::UTF8.GetBytes($needle))
    $encoding.GetString([Text.Encoding]::Unicode.GetBytes($needle))
}
function Test-Content([byte[]]$Bytes, [string]$Label) {
    $raw = $encoding.GetString($Bytes)
    foreach ($needle in $patterns) {
        if ($raw.IndexOf($needle, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
            throw "Local build path found in $Label"
        }
    }
}
foreach ($file in $Files) {
    $item = Get-Item -LiteralPath $file
    Test-Content ([IO.File]::ReadAllBytes($item.FullName)) $item.Name
    if ($item.Extension -in '.apk', '.zip') {
        $archive = [IO.Compression.ZipFile]::OpenRead($item.FullName)
        try {
            foreach ($entry in $archive.Entries) {
                if (!$entry.Length) { continue }
                $stream = $entry.Open()
                $buffer = New-Object IO.MemoryStream
                try {
                    $stream.CopyTo($buffer)
                    Test-Content ($buffer.ToArray()) "$($item.Name)/$($entry.FullName)"
                } finally { $buffer.Dispose(); $stream.Dispose() }
            }
        } finally { $archive.Dispose() }
    }
    Write-Host "Build path check passed: $($item.Name)"
}
