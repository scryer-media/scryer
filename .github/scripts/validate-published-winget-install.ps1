param(
  [Parameter(Mandatory = $true)]
  [string]$ManifestRoot,

  [Parameter(Mandatory = $true)]
  [string]$ExpectedVersion
)

$ErrorActionPreference = "Stop"
$packageId = "ScryerMedia.Scryer"

function Get-ProgramFiles64 {
  if ($env:ProgramW6432) {
    return $env:ProgramW6432
  }

  return ${env:ProgramFiles}
}

function Assert-PublishedMsiInstallation {
  $installDir = Join-Path (Get-ProgramFiles64) "Scryer Media\Scryer"
  $scryerExe = Join-Path $installDir "scryer.exe"
  $trayExe = Join-Path $installDir "scryer-tray.exe"
  foreach ($required in @($scryerExe, $trayExe, (Join-Path $installDir "LICENSE"))) {
    if (-not (Test-Path $required)) {
      throw "winget installed the MSI but did not install $required"
    }
  }
  if (Get-Process scryer-tray -ErrorAction SilentlyContinue) {
    throw "silent winget install started scryer-tray.exe."
  }
  if (Get-CimInstance Win32_Service | Where-Object { $_.PathName -match [regex]::Escape($installDir) }) {
    throw "winget-installed Scryer registered a Windows service."
  }

  $versionOutput = (& $scryerExe --version | Out-String).Trim()
  if ($LASTEXITCODE -ne 0) {
    throw "winget-installed scryer.exe --version failed with exit code $LASTEXITCODE."
  }
  if ($versionOutput -notmatch [regex]::Escape($ExpectedVersion)) {
    throw "winget-installed Scryer reported '$versionOutput', expected version $ExpectedVersion."
  }
}

$winget = (Get-Command winget.exe -ErrorAction SilentlyContinue).Source
if (-not $winget) {
  throw "winget.exe was not found; published MSI install validation is required."
}
if (-not (Test-Path $ManifestRoot)) {
  throw "WinGet manifest root does not exist: $ManifestRoot"
}

& $winget settings --enable LocalManifestFiles
if ($LASTEXITCODE -ne 0) {
  throw "Unable to enable local manifest files in winget (exit code $LASTEXITCODE)."
}
& $winget validate --manifest $ManifestRoot --disable-interactivity
if ($LASTEXITCODE -ne 0) {
  throw "Generated winget manifest validation failed with exit code $LASTEXITCODE."
}

$desktopProfile = Join-Path $env:LOCALAPPDATA "ScryerMedia\Scryer"
$profileMarker = Join-Path $desktopProfile "preserve-on-uninstall.txt"
New-Item -ItemType Directory -Force -Path $desktopProfile | Out-Null
"preserve me" | Set-Content $profileMarker

$installed = $false
try {
  & $winget install --manifest $ManifestRoot --silent --accept-package-agreements --accept-source-agreements --disable-interactivity
  if ($LASTEXITCODE -ne 0) {
    throw "winget install of the release MSI failed with exit code $LASTEXITCODE."
  }
  $installed = $true
  Assert-PublishedMsiInstallation
} finally {
  if ($installed) {
    & $winget uninstall --id $packageId --exact --silent --accept-source-agreements --disable-interactivity
    if ($LASTEXITCODE -ne 0) {
      throw "winget cleanup failed with exit code $LASTEXITCODE."
    }
    if (-not (Test-Path $profileMarker)) {
      throw "MSI uninstall removed Scryer desktop user data at $profileMarker."
    }
  }
}
