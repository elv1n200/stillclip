param([string]$OutputDir = 'models')
$ErrorActionPreference = 'Stop'
$manifest = Get-Content (Join-Path $PSScriptRoot 'ocr-models.json') -Raw | ConvertFrom-Json
New-Item -ItemType Directory -Force $OutputDir | Out-Null
foreach ($model in $manifest.models) {
    $path = Join-Path $OutputDir $model.file
    if ((Test-Path $path) -and (Get-FileHash $path -Algorithm SHA256).Hash -eq $model.sha256) { continue }
    $temporary = "$path.download"
    try {
        Invoke-WebRequest $model.url -OutFile $temporary
        if ((Get-Item $temporary).Length -ne $model.bytes -or (Get-FileHash $temporary -Algorithm SHA256).Hash -ne $model.sha256) {
            throw "OCR model verification failed: $($model.file)"
        }
        Move-Item $temporary $path -Force
    } finally { Remove-Item $temporary -Force -ErrorAction SilentlyContinue }
}
Copy-Item (Join-Path $PSScriptRoot 'ocr-models.json') $OutputDir
Copy-Item (Join-Path $PSScriptRoot 'OCR-NOTICE.txt') $OutputDir
Copy-Item (Join-Path $PSScriptRoot 'CC-BY-SA-4.0.txt') $OutputDir
