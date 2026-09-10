param(
    [string]$OutputDirectory = "dist"
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

cargo build --release --lib

$source = Join-Path $PSScriptRoot "target\release\my_wry.dll"
$sourceStub = Join-Path $PSScriptRoot "my_wry.pyi"
$sourceBridge = Join-Path $PSScriptRoot "python_bridge.js"
$destinationDirectory = Join-Path $PSScriptRoot $OutputDirectory
$destination = Join-Path $destinationDirectory "my_wry.pyd"
$destinationStub = Join-Path $destinationDirectory "my_wry.pyi"
$destinationBridge = Join-Path $destinationDirectory "python_bridge.js"

New-Item -ItemType Directory -Force -Path $destinationDirectory | Out-Null
Copy-Item -Force -LiteralPath $source -Destination $destination
Copy-Item -Force -LiteralPath $sourceStub -Destination $destinationStub
Copy-Item -Force -LiteralPath $sourceBridge -Destination $destinationBridge

Write-Host "已生成：$destination"
Write-Host "已生成：$destinationStub"
Write-Host "已生成：$destinationBridge"
