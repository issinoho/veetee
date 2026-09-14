# Signing the Windows build

Release builds are signed when the repository has a code-signing certificate. Without one the
release still builds, unsigned, and SmartScreen asks users to confirm the first run.

1. Obtain an Authenticode code-signing certificate (OV or EV) from a certificate authority, as a
   `.pfx` file with its password. (EV certificates on hardware tokens cannot be exported; use a
   cloud signing service such as Azure Trusted Signing instead and adapt the signing step.)
2. In the GitHub repository settings, under *Secrets and variables → Actions*, add:
   - `WINDOWS_CERTIFICATE`: the `.pfx` file encoded as base64, for example
     `base64 -w0 veetee.pfx` on Linux or
     `[Convert]::ToBase64String([IO.File]::ReadAllBytes("veetee.pfx"))` in PowerShell;
   - `WINDOWS_CERTIFICATE_PASSWORD`: its password.
3. Tag a release. The *Sign the executables* step of `.github/workflows/release.yml` signs
   `veetee.exe` and `vt-headless.exe` with SHA-256 and a DigiCert timestamp before the zip is made.

To check a downloaded build: right-click `bin\veetee.exe` → *Properties* → *Digital Signatures*,
or run `signtool verify /pa bin\veetee.exe`.
