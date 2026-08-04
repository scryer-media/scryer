[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)]
  [ValidateSet("x64", "arm64")]
  [string]$Architecture,

  [Parameter(Mandatory = $true)]
  [string]$StageDir,

  [Parameter(Mandatory = $true)]
  [string]$Version,

  [Parameter(Mandatory = $true)]
  [string]$OutputPath
)

$ErrorActionPreference = "Stop"

$upgradeCodes = @{
  x64 = "{70933318-EC5C-487D-9E66-3A981C02AA11}"
  arm64 = "{8F04FA95-5EFB-42CB-985E-8F9C40D922FA}"
}

# Component GUIDs remain stable for an architecture, while x64 and ARM64 retain
# independent component identity so Windows Installer can service both correctly.
$componentGuids = @{
  x64 = @{
    applicationFiles = "{E1706C30-F14A-4906-8483-3897859343A9}"
    startMenuShortcuts = "{4E933BEA-0C43-4A92-9ACD-41CBA8FECD29}"
  }
  arm64 = @{
    applicationFiles = "{A7302B32-804C-4792-927E-448A4ADE6598}"
    startMenuShortcuts = "{70B2930E-6479-4ECC-B79D-CEE65EF15098}"
  }
}

if ($Version -notmatch '^\d+\.\d+\.\d+$') {
  throw "Windows MSI version must be release-derived major.minor.patch, got '$Version'."
}

$stageDir = (Resolve-Path -LiteralPath $StageDir).Path
foreach ($required in @("scryer.exe", "scryer-tray.exe", "LICENSE")) {
  if (-not (Test-Path (Join-Path $stageDir $required))) {
    throw "MSI staging directory is missing ${required}: $stageDir"
  }
}

$wix = Get-Command wix.exe -ErrorAction SilentlyContinue
if (-not $wix) {
  $wix = Get-Command wix -ErrorAction SilentlyContinue
}
if (-not $wix) {
  throw "WiX v4 CLI was not found on PATH. Install the pinned wix dotnet tool before packaging."
}

$source = Join-Path $PSScriptRoot "scryer.wxs"
$outputPath = [System.IO.Path]::GetFullPath($OutputPath)
$outputDir = Split-Path -Parent $outputPath
New-Item -ItemType Directory -Force -Path $outputDir | Out-Null

& $wix.Source build `
  -arch $Architecture `
  -d "StageDir=$stageDir" `
  -d "ProductVersion=$Version" `
  -d "UpgradeCode=$($upgradeCodes[$Architecture])" `
  -d "ApplicationFilesComponentGuid=$($componentGuids[$Architecture].applicationFiles)" `
  -d "StartMenuShortcutsComponentGuid=$($componentGuids[$Architecture].startMenuShortcuts)" `
  -o $outputPath `
  $source
if ($LASTEXITCODE -ne 0) {
  throw "WiX failed to build $outputPath with exit code $LASTEXITCODE."
}

$installer = New-Object -ComObject WindowsInstaller.Installer
$database = $installer.OpenDatabase($outputPath, 0)
$view = $database.OpenView("SELECT `Value` FROM `Property` WHERE `Property`='ProductCode'")
$view.Execute()
$record = $view.Fetch()
if (-not $record) {
  throw "WiX MSI did not contain a ProductCode property: $outputPath"
}
$productCode = $record.StringData(1)
$view.Close()

[ordered]@{
  architecture = $Architecture
  product_code = $productCode
  upgrade_code = $upgradeCodes[$Architecture]
  version = $Version
} | ConvertTo-Json | Set-Content -Encoding utf8 "$outputPath.json"

Write-Host "Built $outputPath with ProductCode $productCode"
