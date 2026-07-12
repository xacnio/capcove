<#
Copy this file to scripts/.store-credentials.ps1 (already gitignored), fill in the
values from Partner Center, then dot-source it before running submit-store.ps1:

    . .\scripts\.store-credentials.ps1
    .\scripts\submit-store.ps1 -Commit
#>
$env:STORE_TENANT_ID = "00000000-0000-0000-0000-000000000000"
$env:STORE_CLIENT_ID = "00000000-0000-0000-0000-000000000000"
$env:STORE_CLIENT_SECRET = "your-azure-ad-app-client-secret"
$env:STORE_APP_ID = "your-partner-center-application-id"
