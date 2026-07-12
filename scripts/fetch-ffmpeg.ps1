<#
.SYNOPSIS
    Downloads the pinned ffmpeg/ffprobe build into src-tauri/binaries/ — the
    same one release.yml/build-artifacts.yml fetch in CI. Not committed to
    git (~137MB each, over GitHub's 100MB per-file push limit), so this is a
    one-time step before `npm run tauri dev`/`build` will work.

.PARAMETER Arch
    "x64" or "arm64". Default: x64.

.EXAMPLE
    .\scripts\fetch-ffmpeg.ps1
    .\scripts\fetch-ffmpeg.ps1 -Arch arm64
#>
param(
    [ValidateSet("x64", "arm64")]
    [string]$Arch = "x64"
)

$ErrorActionPreference = "Stop"

$RepoRoot = Split-Path -Parent $PSScriptRoot
$BinariesDir = Join-Path $RepoRoot "src-tauri\binaries"

# Pinned BtbN/FFmpeg-Builds release — a dated tag, not the rolling `latest`
# one (which gets its assets overwritten in place). Bump alongside
# src-tauri/src/integrity.rs's hash pins and release.yml/build-artifacts.yml's
# download step when upgrading.
$Tag = "autobuild-2026-07-12-13-16"
$Build = "N-125551-ga09be9b91e"

$ArchInfo = @{
    x64   = @{ Name = "win64";    Target = "x86_64-pc-windows-msvc";  ZipSha256 = "b6caec1787d2083be5a6755adb7b650bc23e9fef954742608a241b2d87a0af2d"; FfmpegSha256 = "b6e37c0e4bf1c18bd019e8926c5809ecf734249bd48c227efacf019bd6528b92"; FfprobeSha256 = "fa86ff8d675b24b31979d65da1f569a9f0418e69db184ec6688f3c6d7f9ffa14" }
    arm64 = @{ Name = "winarm64"; Target = "aarch64-pc-windows-msvc"; ZipSha256 = "340c9361a286824839b25ece701389d8d4d94c01b0c4f85016f482ecae2456eb"; FfmpegSha256 = "dcb8e91a5dc0ed2cd29e8a4bf33fc5cf074996094818b0acc1bb2616b52381f9"; FfprobeSha256 = "0fdb93d7771e8db63ddc0469cf4e151e53a0ffb9f2dfe1e50a9654da6fa5d680" }
}
$Info = $ArchInfo[$Arch]

function Assert-Sha256([string]$Path, [string]$Expected) {
    $actual = (Get-FileHash -Path $Path -Algorithm SHA256).Hash
    if ($actual -ne $Expected.ToUpper()) {
        throw "Hash mismatch for $Path`nexpected: $Expected`nactual:   $actual"
    }
}

$TempDir = Join-Path ([System.IO.Path]::GetTempPath()) "capcove-ffmpeg-fetch"
if (Test-Path $TempDir) { Remove-Item $TempDir -Recurse -Force }
New-Item -ItemType Directory -Path $TempDir -Force | Out-Null

try {
    $ZipName = "ffmpeg-$Build-$($Info.Name)-gpl.zip"
    $ZipPath = Join-Path $TempDir $ZipName
    Write-Host "==> Downloading $ZipName ($Arch)" -ForegroundColor Cyan
    Invoke-WebRequest "https://github.com/BtbN/FFmpeg-Builds/releases/download/$Tag/$ZipName" -OutFile $ZipPath

    Write-Host "==> Verifying archive checksum" -ForegroundColor Cyan
    Assert-Sha256 $ZipPath $Info.ZipSha256

    Write-Host "==> Extracting" -ForegroundColor Cyan
    $ExtractDir = Join-Path $TempDir "extracted"
    Expand-Archive $ZipPath -DestinationPath $ExtractDir -Force
    $BinDir = Split-Path (Get-ChildItem $ExtractDir -Recurse -Filter "ffmpeg.exe" | Select-Object -First 1).FullName

    New-Item -ItemType Directory -Path $BinariesDir -Force | Out-Null
    $FfmpegDest = Join-Path $BinariesDir "ffmpeg-$($Info.Target).exe"
    $FfprobeDest = Join-Path $BinariesDir "ffprobe-$($Info.Target).exe"
    Copy-Item (Join-Path $BinDir "ffmpeg.exe") $FfmpegDest -Force
    Copy-Item (Join-Path $BinDir "ffprobe.exe") $FfprobeDest -Force

    Write-Host "==> Verifying extracted binaries" -ForegroundColor Cyan
    Assert-Sha256 $FfmpegDest $Info.FfmpegSha256
    Assert-Sha256 $FfprobeDest $Info.FfprobeSha256

    Write-Host "==> Done: $FfmpegDest" -ForegroundColor Green
    Write-Host "==> Done: $FfprobeDest" -ForegroundColor Green
} finally {
    Remove-Item $TempDir -Recurse -Force -ErrorAction SilentlyContinue
}
