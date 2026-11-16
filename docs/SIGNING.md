# Signing releases

Stillclip's packaging can sign the application, both installer variants, and
the Inno-generated uninstallers. Signing is optional; without an owner-provided
certificate, the build produces unsigned packages and does not claim a verified
Windows publisher. No private keys are stored in this repository.

## Windows certificate store

Use a publicly trusted code-signing certificate belonging to the project owner,
with its private key available through an installed Windows CSP/KSP. Modern
providers may require a hardware token, HSM, or a signing-service client. Install
that provider's software and complete its identity verification yourself. A
self-signed certificate does not establish a publicly trusted publisher.

On the signing machine, install the Windows SDK signing tools and Inno Setup
6.5 or later. Build the release and obtain the media artifacts described in
[Installation](INSTALLATION.md), then run:

```powershell
./packaging/build-installers.ps1 `
  -MediaDir media-artifacts `
  -CertificateThumbprint 'YOUR_40_CHARACTER_CERTIFICATE_THUMBPRINT'
```

Replace the placeholder with the certificate's public SHA-1 thumbprint from
`Cert:\CurrentUser\My`. The script validates its lifetime, private-key availability
and code-signing usage. It signs using SHA-256 with an RFC 3161 timestamp, checks
Windows' signature status, and fails the build if signing or verification fails.
The thumbprint selects the certificate; file and timestamp signatures use SHA-256.

The app is signed before it enters the payload, so the portable ZIP gets the
same signed EXE. Inno's `SignTool` integration signs the uninstaller and each
installer during compilation. `SHA256SUMS.txt` is generated after signing.
Never sign a published asset in place: publish a new verified release instead.

## CI and signing services

The ordinary verification workflow intentionally has no signing credentials.
To enable trusted signing in automation, provision an owner-controlled Windows
runner with the signing provider and certificate access, then supply the public
thumbprint to the same build command. Restrict certificate access to a protected
release job. Do not expose signing access to pull requests or untrusted source.
Hardware-backed providers may need an interactive authorization step; follow the
provider's documented unattended-signing procedure where supported.

Cloud signing services that do not expose a Windows certificate-store provider
need their own authenticated adapter. Their account, verified identity and
service-specific configuration cannot be inferred from the repository. Do not
paste private keys, certificate passwords or signing tokens into chat or Git.

Before publishing, verify the installed EXE, both setup files and `unins000.exe`
with `Get-AuthenticodeSignature`, including publisher and timestamp, then rerun
installation, upgrade and uninstall checks on a disposable Windows machine.
The standard pipeline's unsigned installer checks do not establish that a real
certificate-backed release has been signed successfully.

## Updates

A signed installed Stillclip only launches an update installer with a valid
Authenticode signature and the same certificate subject. This permits routine
certificate renewal under the same publisher identity. A publisher identity
change requires a deliberate manual update. SHA-256 checks authenticate bytes
against the GitHub release channel; they do not create a publisher identity or
guarantee SmartScreen reputation.
