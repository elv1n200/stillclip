param(
    [Parameter(Mandatory)][string]$File,
    [Parameter(Mandatory)][ValidatePattern('^[0-9a-fA-F]{40}$')][string]$CertificateThumbprint,
    [string]$SignTool = '',
    [string]$TimestampUrl = 'http://timestamp.digicert.com'
)
$ErrorActionPreference = 'Stop'
if (!$SignTool) {
    $SignTool = (Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" |
        Sort-Object FullName -Descending | Select-Object -First 1).FullName
}
if (!$SignTool -or !(Test-Path $SignTool)) { throw 'Install the Windows SDK signing tools first.' }
$certificate = Get-Item "Cert:\CurrentUser\My\$CertificateThumbprint" -ErrorAction Stop
if (!$certificate.HasPrivateKey -or $certificate.NotAfter -lt (Get-Date) -or $certificate.NotBefore -gt (Get-Date)) {
    throw 'The signing certificate is expired, not yet valid, or has no accessible private key.'
}
$purposes = @($certificate.Extensions |
    Where-Object { $_ -is [System.Security.Cryptography.X509Certificates.X509EnhancedKeyUsageExtension] } |
    ForEach-Object { $_.EnhancedKeyUsages } | ForEach-Object { $_.Value })
if ($purposes -notcontains '1.3.6.1.5.5.7.3.3') { throw 'A code-signing certificate is required.' }
$path = (Resolve-Path -LiteralPath $File).Path
# Hardware-backed certificates work through their installed Windows CSP/KSP.
# The key stays in its provider. No private key or password is exported.
& $SignTool sign /sha1 $CertificateThumbprint /fd SHA256 /tr $TimestampUrl /td SHA256 $path
if ($LASTEXITCODE -ne 0) { throw "Signing failed for $path" }
$signature = Get-AuthenticodeSignature -LiteralPath $path
if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Thumbprint -ne $CertificateThumbprint -or !$signature.TimeStamperCertificate) {
    throw "Signature or trusted timestamp verification failed for $path"
}
