Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repo = if ($env:THRONGLETS_INSTALL_REPO) { $env:THRONGLETS_INSTALL_REPO } else { "Shangri-la-0428/Thronglets" }
$version = if ($env:THRONGLETS_VERSION) { $env:THRONGLETS_VERSION } else { "latest" }
$installDir = if ($env:THRONGLETS_INSTALL_DIR) { $env:THRONGLETS_INSTALL_DIR } else { Join-Path $HOME ".local\bin" }
$asset = "thronglets-mcp-windows-amd64.exe"
$binPath = Join-Path $installDir "thronglets.exe"

if ($version -eq "latest") {
  $url = "https://github.com/$repo/releases/latest/download/$asset"
} else {
  $url = "https://github.com/$repo/releases/download/v$version/$asset"
}

New-Item -ItemType Directory -Force -Path $installDir | Out-Null
Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $binPath

Write-Host "Installed thronglets to $binPath"
Write-Host "Next: thronglets setup"
