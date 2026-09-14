# Signing the Windows build

veetee's Windows executables are signed with a Certum code-signing certificate held in Certum's
SimplySign cloud. The key cannot be exported, so signing happens on a Windows computer signed in to
SimplySign, after the Release workflow has published the unsigned build.

## Signing a release

On Windows, with:

- **SimplySign Desktop** installed, running and signed in with a code from the SimplySign mobile
  app. It presents the certificate in the *Personal* certificate store through a virtual card.
- **signtool** from the Windows SDK
  (`winget install Microsoft.WindowsSDK.10.0.26100`, or the SDK installer with *Signing Tools*).
- **PowerShell 7** (`winget install Microsoft.PowerShell`).
- The **GitHub CLI**, signed in with access to the repository
  (`winget install GitHub.cli`, then `gh auth login`).

Once the Release workflow has finished, from a clone of the repository:

```powershell
pwsh -File packaging\windows\sign-release.ps1 -Version 0.8.1
```

The script:

1. downloads `veetee-0.8.1-x86_64-windows.zip` and `SHA256SUMS` from the GitHub release and checks
   the zip against the published checksum;
2. signs `bin\veetee.exe` and `bin\vt-headless.exe` with SHA-256 and a Certum timestamp
   (`http://time.certum.pl`), then verifies both signatures;
3. rebuilds the zip with exactly the same contents and layout;
4. replaces the zip and `SHA256SUMS` on the release.

SimplySign may ask for approval on the phone while signing. Use `-NoUpload` to sign and rebuild
the zip locally without changing the release, and `-Thumbprint` to choose a certificate when the
store holds more than one code-signing certificate.

The GTK and other DLLs in the zip are third-party libraries and are left as they are.

## Checking a signature

Right-click `bin\veetee.exe` → *Properties* → *Digital Signatures*, or run
`signtool verify /pa /v bin\veetee.exe`. SmartScreen may still warn about a newly signed program
until it has been downloaded enough times to build a reputation.

## Signing in CI instead

For a certificate that can be exported as a `.pfx` file (not SimplySign or a hardware token), the
Release workflow can sign unattended: add the repository secrets `WINDOWS_CERTIFICATE` (the `.pfx`
encoded as base64) and `WINDOWS_CERTIFICATE_PASSWORD`, and its *Sign the executables* step signs
before the zip is made. Without those secrets the step is skipped.
