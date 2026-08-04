param(
  [string]$Destination = (Join-Path $PSScriptRoot "..\src-tauri\resources\libmpv-2.dll")
)

$ErrorActionPreference = "Stop"
$runtimeUrl = "https://github.com/Opiiie/SyncWatch/releases/download/runtime-v1/libmpv-2.dll"
$expectedSha256 = "B7CE1D6145DD86BE99B3EB04CD4307D484F22F1B957104C0C437B14999451BD2"
$destinationPath = [System.IO.Path]::GetFullPath($Destination)
$destinationDirectory = Split-Path -Parent $destinationPath
$temporaryPath = "$destinationPath.download"

New-Item -ItemType Directory -Force -Path $destinationDirectory | Out-Null
try {
  Invoke-WebRequest -Uri $runtimeUrl -OutFile $temporaryPath
  $actualSha256 = (Get-FileHash -LiteralPath $temporaryPath -Algorithm SHA256).Hash
  if ($actualSha256 -ne $expectedSha256) {
    throw "Проверка libmpv не пройдена: контрольная сумма не совпадает."
  }
  Move-Item -LiteralPath $temporaryPath -Destination $destinationPath -Force
  Write-Host "libmpv подготовлена для сборки."
} finally {
  if (Test-Path -LiteralPath $temporaryPath) {
    Remove-Item -LiteralPath $temporaryPath -Force
  }
}
