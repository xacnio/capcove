<#
.SYNOPSIS
    Builds Store-ready MSIX packages (via build-msix.ps1) and pushes them through the
    Microsoft Store Submission API, optionally committing for certification.

.DESCRIPTION
    Requires an Azure AD app registered for Partner Center API access, with these
    values supplied as environment variables (never as script params, to keep
    secrets out of shell history):

        STORE_TENANT_ID      Azure AD tenant ID
        STORE_CLIENT_ID      Azure AD application (client) ID
        STORE_CLIENT_SECRET  Azure AD application client secret
        STORE_APP_ID         The app's Store ID (Partner Center app identity page,
                              NOT an Azure AD GUID)

    -ReleaseNotesFile (default scripts/release-notes.json) supplies per-language
    "What's new" text, e.g. { "en-us": "...", "tr-tr": "..." }. Unknown language
    keys are skipped with a warning.

    Without -Commit, the script stops after updating the submission and prints a
    Partner Center link for manual review. Pass -Commit to submit for certification.

.EXAMPLE
    .\scripts\submit-store.ps1
    .\scripts\submit-store.ps1 -Commit
    .\scripts\submit-store.ps1 -Architectures x64 -SkipBuild -Commit
#>
param(
    [string[]]$Architectures = @("x64", "arm64"),
    [switch]$SkipBuild,
    [switch]$Commit,
    [string]$NotesForCertification = "",
    [string]$ReleaseNotesFile = "",
    [int]$PollIntervalSeconds = 30,
    [int]$PollTimeoutMinutes = 60
)

$ErrorActionPreference = "Stop"

$RepoRoot = Split-Path -Parent $PSScriptRoot
$OutDir = Join-Path $RepoRoot "src-tauri\target\msix-out"
$ZipPath = Join-Path $RepoRoot "src-tauri\target\msix-out\store-submission.zip"
if (-not $ReleaseNotesFile) { $ReleaseNotesFile = Join-Path $PSScriptRoot "release-notes.json" }

function Get-RequiredEnv([string]$Name) {
    $value = [System.Environment]::GetEnvironmentVariable($Name)
    if ([string]::IsNullOrWhiteSpace($value)) {
        throw "Missing required environment variable: $Name (see scripts/submit-store.ps1 header for the full list)."
    }
    return $value
}

$TenantId = Get-RequiredEnv "STORE_TENANT_ID"
$ClientId = Get-RequiredEnv "STORE_CLIENT_ID"
$ClientSecret = Get-RequiredEnv "STORE_CLIENT_SECRET"
$AppId = Get-RequiredEnv "STORE_APP_ID"

$ApiBase = "https://manage.devcenter.microsoft.com/v1.0/my/applications/$AppId"

Write-Host "==> Authenticating to Azure AD" -ForegroundColor Cyan
$tokenResponse = Invoke-RestMethod -Method Post -Uri "https://login.microsoftonline.com/$TenantId/oauth2/token" -Body @{
    grant_type    = "client_credentials"
    client_id     = $ClientId
    client_secret = $ClientSecret
    resource      = "https://manage.devcenter.microsoft.com"
}
$AuthHeaders = @{
    Authorization  = "Bearer $($tokenResponse.access_token)"
    "Content-Type" = "application/json"
}

if (-not $SkipBuild) {
    foreach ($arch in $Architectures) {
        Write-Host "==> Packaging $arch" -ForegroundColor Cyan
        & (Join-Path $PSScriptRoot "build-msix.ps1") -Arch $arch
    }
}

$TauriConf = Get-Content (Join-Path $RepoRoot "src-tauri\tauri.conf.json") -Raw | ConvertFrom-Json
$MsixVersion = "$($TauriConf.version).0"

$PackageFiles = Get-ChildItem $OutDir -Filter "*_${MsixVersion}_*.msix" | Sort-Object Name
if ($PackageFiles.Count -eq 0) { throw "No v$MsixVersion .msix files found in $OutDir. Build first or drop -SkipBuild." }
Write-Host "==> Packages to submit:" -ForegroundColor Cyan
$PackageFiles | ForEach-Object { Write-Host "    $($_.Name)" }

Write-Host "==> Checking for an existing pending submission" -ForegroundColor Cyan
$app = Invoke-RestMethod -Method Get -Uri $ApiBase -Headers $AuthHeaders
$submission = $null
if ($app.pendingApplicationSubmission -and $app.pendingApplicationSubmission.id) {
    $pendingId = $app.pendingApplicationSubmission.id
    $pendingStatus = Invoke-RestMethod -Method Get -Uri "$ApiBase/submissions/$pendingId/status" -Headers $AuthHeaders
    # "None"/"PendingCommit" means still a draft, safe to reuse. Any other status
    # means commit already succeeded and the Store is processing it.
    if ($pendingStatus.status -notin @("None", "PendingCommit")) {
        throw "App $AppId already has a submission ($pendingId) in status '$($pendingStatus.status)' - it's already past draft and being processed by the Store. Wait for it to finish or cancel it manually in Partner Center before running this script."
    }
    Write-Host "    Reusing existing draft submission $pendingId (status: $($pendingStatus.status))" -ForegroundColor Yellow
    $submission = Invoke-RestMethod -Method Get -Uri "$ApiBase/submissions/$pendingId" -Headers $AuthHeaders
}

if (-not $submission) {
    Write-Host "==> Creating submission (clone of last published)" -ForegroundColor Cyan
    $submission = Invoke-RestMethod -Method Post -Uri "$ApiBase/submissions" -Headers $AuthHeaders
}
$SubmissionId = $submission.id
Write-Host "    Submission ID: $SubmissionId"

Write-Host "==> Building submission zip" -ForegroundColor Cyan
if (Test-Path $ZipPath) { Remove-Item $ZipPath -Force }
Compress-Archive -Path $PackageFiles.FullName -DestinationPath $ZipPath

Write-Host "==> Uploading package zip" -ForegroundColor Cyan
$zipBytes = [System.IO.File]::ReadAllBytes($ZipPath)
Invoke-WebRequest -Method Put -Uri $submission.fileUploadUrl -Body $zipBytes -UseBasicParsing `
    -Headers @{ "x-ms-blob-type" = "BlockBlob" } -ContentType "application/zip" | Out-Null

Write-Host "==> Updating submission package list" -ForegroundColor Cyan
# A reused draft may already have these packages with a server-assigned id;
# leave those untouched, since re-adding or PendingDelete-ing them makes the API reject the PUT.
$desiredNames = @($PackageFiles.Name)
$existingPackages = @($submission.applicationPackages | ForEach-Object {
    if ($desiredNames -notcontains $_.fileName) {
        $_.fileStatus = "PendingDelete"
    }
    $_
})
$alreadyPresentNames = @($submission.applicationPackages.fileName)
$newPackages = @($PackageFiles | Where-Object { $alreadyPresentNames -notcontains $_.Name } | ForEach-Object {
    [PSCustomObject]@{ fileName = $_.Name; fileStatus = "PendingUpload" }
})
$submission.applicationPackages = @($existingPackages + $newPackages)
if ($NotesForCertification) { $submission.notesForCertification = $NotesForCertification }

if (Test-Path $ReleaseNotesFile) {
    Write-Host "==> Applying release notes from $ReleaseNotesFile" -ForegroundColor Cyan
    $releaseNotes = Get-Content $ReleaseNotesFile -Raw -Encoding UTF8 | ConvertFrom-Json
    foreach ($prop in $releaseNotes.PSObject.Properties) {
        $lang = $prop.Name
        $listingProp = $submission.listings.PSObject.Properties[$lang]
        if (-not $listingProp) {
            Write-Host "    Skipping '$lang' - no matching listing language on this submission" -ForegroundColor Yellow
            continue
        }
        $submission.listings.$lang.baseListing.releaseNotes = $prop.Value
        Write-Host "    Set releaseNotes for $lang"
    }
}

if ($submission.pricing -and $submission.pricing.priceId -eq "Base") {
    # Known API bug: a cloned submission's pricing.priceId comes back as "Base",
    # which the API rejects if PUT back unchanged, so clear just that field.
    $submission.pricing.priceId = $null
}

$submissionJson = $submission | ConvertTo-Json -Depth 20
$submissionJsonBytes = [System.Text.Encoding]::UTF8.GetBytes($submissionJson)
Write-Host "    Sending applicationPackages: $(($submission.applicationPackages | ForEach-Object { "$($_.fileName):$($_.fileStatus)" }) -join ', ')" -ForegroundColor DarkGray
$updated = Invoke-RestMethod -Method Put -Uri "$ApiBase/submissions/$SubmissionId" -Headers @{ Authorization = $AuthHeaders.Authorization } -Body $submissionJsonBytes -ContentType "application/json; charset=utf-8"
Write-Host "    PUT response applicationPackages: $(($updated.applicationPackages | ForEach-Object { "$($_.fileName):$($_.fileStatus)" }) -join ', ')" -ForegroundColor DarkGray

Write-Host "==> Re-fetching submission to confirm it actually persisted" -ForegroundColor Cyan
$verify = Invoke-RestMethod -Method Get -Uri "$ApiBase/submissions/$SubmissionId" -Headers $AuthHeaders
Write-Host "    GET applicationPackages: $(($verify.applicationPackages | ForEach-Object { "$($_.fileName):$($_.fileStatus)" }) -join ', ')" -ForegroundColor Yellow
Write-Host "    GET releaseNotes (en-us): $($verify.listings.'en-us'.baseListing.releaseNotes)" -ForegroundColor Yellow

$PartnerCenterUrl = "https://partner.microsoft.com/dashboard/products/$AppId/submissions/$SubmissionId"
Write-Host "==> Submission updated: $PartnerCenterUrl" -ForegroundColor Green

if (-not $Commit) {
    Write-Host "==> Skipping commit (pass -Commit to submit for certification). Review in Partner Center first." -ForegroundColor Yellow
    return
}

Write-Host "==> Committing submission for certification" -ForegroundColor Cyan
Invoke-RestMethod -Method Post -Uri "$ApiBase/submissions/$SubmissionId/commit" -Headers $AuthHeaders | Out-Null

Write-Host "==> Polling status (every $PollIntervalSeconds s, timeout $PollTimeoutMinutes min)" -ForegroundColor Cyan
$deadline = (Get-Date).AddMinutes($PollTimeoutMinutes)
do {
    Start-Sleep -Seconds $PollIntervalSeconds
    $status = Invoke-RestMethod -Method Get -Uri "$ApiBase/submissions/$SubmissionId/status" -Headers $AuthHeaders
    Write-Host "    status=$($status.status)"
    if ($status.statusDetails.errors) {
        $status.statusDetails.errors | ForEach-Object { Write-Host "    ERROR: $($_.message)" -ForegroundColor Red }
    }
} while ($status.status -notin @("Published", "Release", "Failed", "CommitFailed", "Canceled") -and (Get-Date) -lt $deadline)

if ($status.status -in @("Failed", "CommitFailed", "Canceled")) {
    throw "Submission ended with status $($status.status). Check $PartnerCenterUrl for details."
}
Write-Host "==> Done: $($status.status). $PartnerCenterUrl" -ForegroundColor Green
