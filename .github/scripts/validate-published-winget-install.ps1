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

function Get-PublishedMsiProductCode {
  param(
    [Parameter(Mandatory = $true)]
    [string]$ManifestDirectory
  )

  $installerManifest = Join-Path $ManifestDirectory "$packageId.installer.yaml"
  if (-not (Test-Path $installerManifest)) {
    throw "Published WinGet installer manifest was not found at $installerManifest."
  }

  $x64Installer = $false
  foreach ($line in Get-Content -LiteralPath $installerManifest) {
    if ($line -match '^\s*-\s*Architecture:\s*(?<architecture>\S+)\s*$') {
      $x64Installer = $Matches.architecture -eq "x64"
      continue
    }
    if ($x64Installer -and $line -match '^\s*ProductCode:\s*''(?<productCode>\{[0-9A-Fa-f-]+\})''\s*$') {
      return $Matches.productCode
    }
  }

  throw "Published WinGet installer manifest did not declare an x64 MSI ProductCode."
}

function Assert-PublishedMsiRemoval {
  $installDir = Join-Path (Get-ProgramFiles64) "Scryer Media\Scryer"
  if (Test-Path $installDir) {
    throw "MSI cleanup left the Scryer install directory at $installDir."
  }
  if (Get-CimInstance Win32_Service | Where-Object { $_.PathName -match [regex]::Escape($installDir) }) {
    throw "MSI cleanup left a Scryer Windows service registered."
  }
}

$winget = (Get-Command winget.exe -ErrorAction SilentlyContinue).Source
if (-not $winget) {
  throw "winget.exe was not found; published MSI install validation is required."
}
if (-not (Test-Path $ManifestRoot)) {
  throw "WinGet manifest root does not exist: $ManifestRoot"
}

$manifestDirectories = @(
  Get-ChildItem -Path $ManifestRoot -Recurse -File -Filter "*.yaml" |
    ForEach-Object { $_.DirectoryName } |
    Sort-Object -Unique
)
if ($manifestDirectories.Count -ne 1) {
  throw "Expected exactly one directory containing WinGet manifest YAML files below $ManifestRoot; found $($manifestDirectories.Count)."
}
$manifestDirectory = $manifestDirectories[0]

& $winget settings --enable LocalManifestFiles
if ($LASTEXITCODE -ne 0) {
  throw "Unable to enable local manifest files in winget (exit code $LASTEXITCODE)."
}
& $winget validate --manifest $manifestDirectory --disable-interactivity
$manifestValidationExitCode = $LASTEXITCODE
if ($manifestValidationExitCode -eq -1978335192) {
  Write-Warning "Generated winget manifest validation succeeded with warnings. Continuing to the install smoke test."
} elseif ($manifestValidationExitCode -ne 0) {
  throw "Generated winget manifest validation failed with exit code $manifestValidationExitCode."
}

# A real uninstall runs `scryer-tray --uninstall-cleanup`, which removes the
# desktop profile but keeps a backups folder that still holds anything.
$desktopProfile = Join-Path $env:LOCALAPPDATA "ScryerMedia\Scryer"
$profileMarker = Join-Path $desktopProfile "removed-on-uninstall.txt"
$backupMarker = Join-Path $desktopProfile "backups\kept-on-uninstall.txt"
New-Item -ItemType Directory -Force -Path (Split-Path $backupMarker) | Out-Null
"remove me" | Set-Content $profileMarker
"keep me" | Set-Content $backupMarker
$msiProductCode = Get-PublishedMsiProductCode -ManifestDirectory $manifestDirectory

$installed = $false
try {
  & $winget install --manifest $manifestDirectory --silent --accept-package-agreements --accept-source-agreements --disable-interactivity
  if ($LASTEXITCODE -ne 0) {
    throw "winget install of the release MSI failed with exit code $LASTEXITCODE."
  }
  $installed = $true
  Assert-PublishedMsiInstallation
} finally {
  if ($installed) {
    $uninstall = Start-Process -FilePath "$env:SystemRoot\System32\msiexec.exe" -ArgumentList @("/x", $msiProductCode, "/qn", "/norestart") -Wait -PassThru
    if ($uninstall.ExitCode -notin @(0, 3010)) {
      throw "MSI cleanup failed with exit code $($uninstall.ExitCode)."
    }
    Assert-PublishedMsiRemoval
    if (Test-Path $profileMarker) {
      throw "MSI uninstall left Scryer desktop profile data at $profileMarker."
    }
    if (-not (Test-Path $backupMarker)) {
      throw "MSI uninstall removed a Scryer backup at $backupMarker."
    }
  }
}
