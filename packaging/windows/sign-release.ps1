#Requires -Version 7.0
<#
.SYNOPSIS
    Signs the Windows executables of a veetee GitHub release with a Certum
    SimplySign certificate and replaces the release's zip and SHA256SUMS.

.DESCRIPTION
    Run on Windows with:
      - SimplySign Desktop running and signed in (it makes the certificate
        available in the CurrentUser\My store through its virtual card);
      - signtool.exe from the Windows SDK;
      - the GitHub CLI (gh), signed in with permission to edit releases;
      - PowerShell 7 (winget install Microsoft.PowerShell), whose zip support
        writes the forward-slash paths the release's zip uses.

    The script downloads veetee-VERSION-x86_64-windows.zip from the release,
    signs bin\veetee.exe and bin\vt-headless.exe (SHA-256, timestamped by
    Certum), verifies the signatures, rebuilds the zip with the same layout
    and uploads it and an updated SHA256SUMS in place of the originals.

.PARAMETER Version
    The release to sign, for example 0.8.1.

.PARAMETER Thumbprint
    The SHA-1 thumbprint of the code-signing certificate. Without it, the only
    code-signing certificate in CurrentUser\My is used.

.PARAMETER NoUpload
    Signs and rebuilds the zip in the working folder without changing the release.

.EXAMPLE
    .\packaging\windows\sign-release.ps1 -Version 0.8.1
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Version,
    [string]$Thumbprint,
    [string]$Repository = "issinoho/veetee",
    [switch]$NoUpload
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Find-SignTool {
    $cmd = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    $kits = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\bin"
    $found = Get-ChildItem -Path $kits -Filter signtool.exe -Recurse -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -match "\\x64\\" } |
        Sort-Object FullName | Select-Object -Last 1
    if (-not $found) { throw "signtool.exe was not found; install the Windows SDK." }
    return $found.FullName
}

function Find-Certificate([string]$Thumbprint) {
    $codeSigning = "1.3.6.1.5.5.7.3.3"
    $certs = @(Get-ChildItem Cert:\CurrentUser\My | Where-Object {
        $_.HasPrivateKey -and ($_.EnhancedKeyUsageList.ObjectId -contains $codeSigning)
    })
    if ($Thumbprint) {
        $certs = @($certs | Where-Object { $_.Thumbprint -eq $Thumbprint.Replace(" ", "").ToUpper() })
    }
    if ($certs.Count -eq 0) {
        throw "No code-signing certificate found. Is SimplySign Desktop running and signed in?"
    }
    if ($certs.Count -gt 1) {
        $list = ($certs | ForEach-Object { "  $($_.Thumbprint)  $($_.Subject)" }) -join "`n"
        throw "Several code-signing certificates were found; choose one with -Thumbprint:`n$list"
    }
    return $certs[0]
}

foreach ($tool in @("gh")) {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
        throw "$tool was not found on PATH."
    }
}
$signtool = Find-SignTool
$cert = Find-Certificate $Thumbprint
Write-Host "Signing with: $($cert.Subject) ($($cert.Thumbprint)), valid until $($cert.NotAfter)"

$tag = "v$Version"
$name = "veetee-$Version-x86_64-windows"
$zip = "$name.zip"
$work = Join-Path ([IO.Path]::GetTempPath()) "veetee-sign-$Version"
if (Test-Path $work) { Remove-Item -Recurse -Force $work }
New-Item -ItemType Directory -Path $work | Out-Null

Write-Host "Downloading $zip and SHA256SUMS from $Repository $tag"
gh release download $tag --repo $Repository --dir $work --pattern $zip --pattern SHA256SUMS
if ($LASTEXITCODE -ne 0) { throw "gh release download failed" }

# Check the download against the published checksum before signing it.
$sums = Join-Path $work "SHA256SUMS"
$expected = (Select-String -Path $sums -Pattern ([regex]::Escape($zip) + "$")).Line.Split(" ")[0]
$actual = (Get-FileHash -Algorithm SHA256 (Join-Path $work $zip)).Hash.ToLower()
if ($expected -ne $actual) { throw "$zip does not match SHA256SUMS" }

$unpacked = Join-Path $work "unpacked"
Expand-Archive -Path (Join-Path $work $zip) -DestinationPath $unpacked
$bin = Join-Path $unpacked "$name\bin"
$targets = @("veetee.exe", "vt-headless.exe") | ForEach-Object { Join-Path $bin $_ }

& $signtool sign /sha1 $cert.Thumbprint /fd SHA256 /tr http://time.certum.pl /td SHA256 `
    /d "veetee" /du "https://github.com/$Repository" $targets
if ($LASTEXITCODE -ne 0) { throw "signtool sign failed" }
& $signtool verify /pa /v $targets
if ($LASTEXITCODE -ne 0) { throw "signtool verify failed" }

$signedZip = Join-Path (Get-Location) $zip
if (Test-Path $signedZip) { Remove-Item -Force $signedZip }
# Keep the layout exactly, empty folders included, with the top folder in the zip.
[IO.Compression.ZipFile]::CreateFromDirectory((Join-Path $unpacked $name), $signedZip,
    [IO.Compression.CompressionLevel]::Optimal, $true)
$hash = (Get-FileHash -Algorithm SHA256 $signedZip).Hash.ToLower()

$updated = Get-Content $sums | ForEach-Object {
    if ($_ -match ([regex]::Escape($zip) + "$")) { "$hash  $zip" } else { $_ }
}
$newSums = Join-Path (Get-Location) "SHA256SUMS"
# SHA256SUMS keeps Unix line endings so `sha256sum -c` reads it.
[IO.File]::WriteAllText($newSums, (($updated -join "`n") + "`n"))
Write-Host "Signed $zip ($hash)"

if ($NoUpload) {
    Write-Host "Left $zip and SHA256SUMS in $(Get-Location); the release is unchanged."
} else {
    gh release upload $tag --repo $Repository --clobber $signedZip $newSums
    if ($LASTEXITCODE -ne 0) { throw "gh release upload failed" }
    Write-Host "Replaced $zip and SHA256SUMS on $tag."
}
Remove-Item -Recurse -Force $work
