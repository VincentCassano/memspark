param(
    [string]$Version = ""
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot

if ([string]::IsNullOrWhiteSpace($Version)) {
    $cargoToml = Get-Content -LiteralPath (Join-Path $repoRoot "Cargo.toml") -Encoding UTF8
    $versionLine = $cargoToml | Where-Object { $_ -match '^version\s*=\s*"([^"]+)"' } | Select-Object -First 1
    if (-not $versionLine) {
        throw "Cannot find workspace version in Cargo.toml"
    }
    $Version = [regex]::Match($versionLine, '"([^"]+)"').Groups[1].Value
}

$releaseRoot = Join-Path $repoRoot "artifacts\release\v$Version"
$packageName = "MemSpark-v$Version-windows-x64"
$packageDir = Join-Path $releaseRoot $packageName
$guiExe = Join-Path $repoRoot "target\release\memspark-ui.exe"
$cliExe = Join-Path $repoRoot "target\release\memspark.exe"

if (!(Test-Path -LiteralPath $guiExe)) {
    throw "GUI executable not found: $guiExe"
}
if (!(Test-Path -LiteralPath $cliExe)) {
    throw "CLI executable not found: $cliExe"
}

New-Item -ItemType Directory -Path $packageDir -Force | Out-Null

Copy-Item -LiteralPath $guiExe -Destination (Join-Path $packageDir "MemSpark.exe") -Force
Copy-Item -LiteralPath $cliExe -Destination (Join-Path $packageDir "memspark-cli.exe") -Force
Copy-Item -LiteralPath (Join-Path $repoRoot "README.md") -Destination (Join-Path $packageDir "README.md") -Force
Copy-Item -LiteralPath (Join-Path $repoRoot "LICENSE") -Destination (Join-Path $packageDir "LICENSE") -Force
Copy-Item -LiteralPath (Join-Path $repoRoot "NOTICE.md") -Destination (Join-Path $packageDir "NOTICE.md") -Force

$guiAsset = Join-Path $releaseRoot "MemSpark-GUI-v$Version-windows-x64.exe"
$cliAsset = Join-Path $releaseRoot "MemSpark-CLI-v$Version-windows-x64.exe"
$zipAsset = Join-Path $releaseRoot "$packageName.zip"

Copy-Item -LiteralPath $guiExe -Destination $guiAsset -Force
Copy-Item -LiteralPath $cliExe -Destination $cliAsset -Force
if (Test-Path -LiteralPath $zipAsset) {
    Remove-Item -LiteralPath $zipAsset -Force
}
Compress-Archive -LiteralPath $packageDir -DestinationPath $zipAsset -Force

Get-ChildItem -LiteralPath $releaseRoot | Select-Object Name,Length,LastWriteTime
