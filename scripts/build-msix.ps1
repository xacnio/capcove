<#
.SYNOPSIS
    Packs the already-built Capcove release exe into a local .msix, without the
    MSIX Packaging Tool GUI. Run `cargo build --release` / `npm run tauri build` first.

.PARAMETER Arch
    "x64" or "arm64". Selects which release build to pack and sets the manifest's
    ProcessorArchitecture and output filename. Build that arch yourself first. Default: x64.

.PARAMETER Sign
    Sign the package with a local self-signed test certificate (created once, reused after).

.PARAMETER Install
    Install the resulting package on this machine after packing (Add-AppxPackage).

.EXAMPLE
    .\scripts\build-msix.ps1 -Sign -Install
#>
param(
    [ValidateSet("x64", "arm64")]
    [string]$Arch = "x64",
    [switch]$Sign,
    [switch]$Install
)

$ErrorActionPreference = "Stop"

$RepoRoot = Split-Path -Parent $PSScriptRoot
$TauriDir = Join-Path $RepoRoot "src-tauri"
$MsixDir = Join-Path $RepoRoot "scripts\msix"
$CertDir = Join-Path $MsixDir ".cert"
$StagingDir = Join-Path $TauriDir "target\msix-staging"
$OutDir = Join-Path $TauriDir "target\msix-out"

$ArchMsix = @{ x64 = "x64"; arm64 = "arm64" }
$RustTarget = @{ x64 = "x86_64-pc-windows-msvc"; arm64 = "aarch64-pc-windows-msvc" }

# PackageFamilyName's "PublisherId" segment: SHA-256 of the Publisher string's first
# 64 bits, base32-ish encoded. Matches the algorithm Windows itself uses.
function Get-PublisherId([string]$Publisher) {
    $bytes = [System.Text.Encoding]::Unicode.GetBytes($Publisher)
    $hash = [System.Security.Cryptography.SHA256]::Create().ComputeHash($bytes)
    $alphabet = "0123456789abcdefghjkmnpqrstvwxyz"
    $bits = ($hash[0..7] | ForEach-Object { [Convert]::ToString($_, 2).PadLeft(8, '0') }) -join ''
    $bits += '0'
    $result = ""
    for ($i = 0; $i -lt 13; $i++) {
        $index = [Convert]::ToInt32($bits.Substring($i * 5, 5), 2)
        $result += $alphabet[$index]
    }
    return $result
}

function Find-SdkTool([string]$Name) {
    $cmd = Get-Command $Name -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    $found = Get-ChildItem "C:\Program Files (x86)\Windows Kits\10\bin" -Recurse -Filter $Name -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -match "\\x64\\" } |
        Sort-Object FullName -Descending | Select-Object -First 1
    if (-not $found) { throw "$Name not found. Install the Windows SDK (it ships with Visual Studio's 'Desktop development with C++' workload)." }
    return $found.FullName
}

$tauriConf = Get-Content (Join-Path $TauriDir "tauri.conf.json") -Raw | ConvertFrom-Json
$Version = $tauriConf.version
$MsixVersion = "$Version.0"

Write-Host "==> Capcove v$Version ($Arch)" -ForegroundColor Cyan

# Host-arch builds land in target\release; cross-compiled builds land under
# target\<triple>\release instead.
$ExeSrc = Join-Path $TauriDir "target\$($RustTarget[$Arch])\release\capcove.exe"
if (-not (Test-Path $ExeSrc)) {
    $HostExeSrc = Join-Path $TauriDir "target\release\capcove.exe"
    if ($Arch -eq "x64" -and (Test-Path $HostExeSrc)) {
        $ExeSrc = $HostExeSrc
    } else {
        throw "Build output not found at $ExeSrc. Build it first: cargo build --release --target $($RustTarget[$Arch]) (or npm run tauri build -- --target $($RustTarget[$Arch]))."
    }
}

Write-Host "==> Staging payload" -ForegroundColor Cyan
if (Test-Path $StagingDir) { Remove-Item $StagingDir -Recurse -Force }
# Matches the real Store package layout: the exe lives under the VFS Local AppData
# redirect, at the same path the NSIS installer puts it on a real machine — and
# must match AppxManifest.template.xml's Application Executable path exactly.
$VfsExeDir = Join-Path $StagingDir "VFS\Local AppData\Capcove"
New-Item -ItemType Directory -Path $VfsExeDir -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $StagingDir "Assets") | Out-Null

Copy-Item $ExeSrc (Join-Path $VfsExeDir "capcove.exe")

# ffmpeg/ffprobe sidecars: tauri-plugin-shell resolves them by bare name next
# to the exe (ffmpeg.exe/ffprobe.exe), not by the triple-suffixed source
# filename, so rename on copy the same way `tauri build` does.
$BinariesSrcDir = Join-Path $TauriDir "binaries"
foreach ($tool in @("ffmpeg", "ffprobe")) {
    $toolSrc = Join-Path $BinariesSrcDir "$tool-$($RustTarget[$Arch]).exe"
    if (-not (Test-Path $toolSrc)) {
        throw "Missing $toolSrc - see .gitignore for how to fetch the pinned ffmpeg build."
    }
    Copy-Item $toolSrc (Join-Path $VfsExeDir "$tool.exe")
}

# Resource packs (game icon/cover art), matching tauri.conf.json's
# bundle.resources mapping (resources/<file> next to the exe).
$ResourcesSrcDir = Join-Path $TauriDir "resources"
$VfsResourcesDir = Join-Path $VfsExeDir "resources"
New-Item -ItemType Directory -Path $VfsResourcesDir -Force | Out-Null
foreach ($pack in @("game_icons.pack", "game_covers.pack")) {
    $packSrc = Join-Path $ResourcesSrcDir $pack
    if (-not (Test-Path $packSrc)) { throw "Missing $packSrc." }
    Copy-Item $packSrc (Join-Path $VfsResourcesDir $pack)
}

$AssetsSrcDir = Join-Path $MsixDir "Assets"
Get-ChildItem $AssetsSrcDir -Filter "*.png" | ForEach-Object {
    Copy-Item $_.FullName (Join-Path $StagingDir "Assets\$($_.Name)")
}

$Template = Get-Content (Join-Path $MsixDir "AppxManifest.template.xml") -Raw
$Manifest = $Template -replace '\{\{VERSION\}\}', $MsixVersion -replace '\{\{ARCH\}\}', $ArchMsix[$Arch]
Set-Content -Path (Join-Path $StagingDir "AppxManifest.xml") -Value $Manifest -Encoding UTF8

# Indexes the scale/targetsize-qualified asset files so Windows can pick the
# right variant for the display's DPI.
Write-Host "==> Indexing resources (makepri)" -ForegroundColor Cyan
$makepri = Find-SdkTool "makepri.exe"
& $makepri new /pr $StagingDir /cf (Join-Path $MsixDir "priconfig.xml") /of (Join-Path $StagingDir "Resources.pri") /o | Out-Null
if ($LASTEXITCODE -ne 0) { throw "makepri failed with exit code $LASTEXITCODE" }

New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
$ManifestXml = [xml]$Manifest
$IdentityName = $ManifestXml.Package.Identity.Name
$Publisher = $ManifestXml.Package.Identity.Publisher
$PublisherId = Get-PublisherId $Publisher
$MsixName = "${IdentityName}_${MsixVersion}_$($ArchMsix[$Arch])__${PublisherId}.msix"
$MsixPath = Join-Path $OutDir $MsixName

Write-Host "==> Packing $MsixName" -ForegroundColor Cyan
$makeappx = Find-SdkTool "makeappx.exe"
& $makeappx pack /d $StagingDir /p $MsixPath /overwrite
if ($LASTEXITCODE -ne 0) { throw "makeappx failed with exit code $LASTEXITCODE" }

if ($Sign) {
    New-Item -ItemType Directory -Path $CertDir -Force | Out-Null
    $PfxPath = Join-Path $CertDir "capcove-test.pfx"
    $CerPath = Join-Path $CertDir "capcove-test.cer"
    $CertSubject = "CN=AB590003-9108-4489-A869-366AA4C19104"
    $PfxPassword = "capcove"

    if (-not (Test-Path $PfxPath)) {
        Write-Host "==> Creating local test certificate ($CertSubject)" -ForegroundColor Cyan
        $cert = New-SelfSignedCertificate -Type Custom -Subject $CertSubject -KeyUsage DigitalSignature `
            -FriendlyName "Capcove MSIX Test Cert" -CertStoreLocation "Cert:\CurrentUser\My" `
            -TextExtension @("2.5.29.37={text}1.3.6.1.5.5.7.3.3", "2.5.29.19={text}false")
        $securePwd = ConvertTo-SecureString -String $PfxPassword -Force -AsPlainText
        Export-PfxCertificate -Cert $cert -FilePath $PfxPath -Password $securePwd | Out-Null
        Export-Certificate -Cert $cert -FilePath $CerPath | Out-Null
    }

    # Self-signed cert needs to be in both Trusted Root and Trusted People for
    # Add-AppxPackage to accept it; re-checked every run in case a prior run
    # created the cert but couldn't get admin to trust it.
    $alreadyTrusted = Get-ChildItem "Cert:\LocalMachine\Root" -ErrorAction SilentlyContinue |
        Where-Object { $_.Subject -eq $CertSubject }
    if (-not $alreadyTrusted) {
        Write-Host "==> Trusting certificate for local installs (admin required)" -ForegroundColor Yellow
        try {
            Import-Certificate -FilePath $CerPath -CertStoreLocation "Cert:\LocalMachine\Root" -ErrorAction Stop | Out-Null
            Import-Certificate -FilePath $CerPath -CertStoreLocation "Cert:\LocalMachine\TrustedPeople" -ErrorAction Stop | Out-Null
        } catch {
            Write-Host "    Not running as admin - requesting elevation for this one step (UAC prompt)..." -ForegroundColor Yellow
            $importCmd = "Import-Certificate -FilePath '$CerPath' -CertStoreLocation 'Cert:\LocalMachine\Root'; Import-Certificate -FilePath '$CerPath' -CertStoreLocation 'Cert:\LocalMachine\TrustedPeople'"
            $proc = Start-Process powershell -ArgumentList "-NoProfile", "-Command", $importCmd -Verb RunAs -Wait -PassThru
            if ($proc.ExitCode -ne 0) {
                Write-Host "    Elevation failed or was declined. Manually import $CerPath into 'Trusted Root Certification Authorities' and 'Trusted People' to install locally." -ForegroundColor Red
            }
        }
    }

    Write-Host "==> Signing package" -ForegroundColor Cyan
    $signtool = Find-SdkTool "signtool.exe"
    & $signtool sign /fd SHA256 /f $PfxPath /p $PfxPassword $MsixPath
    if ($LASTEXITCODE -ne 0) { throw "signtool failed with exit code $LASTEXITCODE" }
}

Write-Host "==> Done: $MsixPath" -ForegroundColor Green

if ($Install) {
    if (-not $Sign) {
        Write-Host "==> -Install requires a signed, trusted package. Re-run with -Sign too." -ForegroundColor Red
    } else {
        Write-Host "==> Installing" -ForegroundColor Cyan
        Add-AppxPackage -Path $MsixPath
    }
}
