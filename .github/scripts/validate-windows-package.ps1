param(
  [Parameter(Mandatory = $true)]
  [string]$Architecture,

  [Parameter(Mandatory = $true)]
  [string]$ZipPath,

  [Parameter(Mandatory = $true)]
  [string]$TarballPath,

  [Parameter(Mandatory = $true)]
  [string]$MsiPath,

  [Parameter(Mandatory = $true)]
  [string]$MsiMetadataPath,

  [Parameter(Mandatory = $true)]
  [string]$WingetMsiPath,

  [Parameter(Mandatory = $true)]
  [string]$WingetMsiMetadataPath,

  [Parameter(Mandatory = $true)]
  [string]$BuiltExePath,

  [Parameter(Mandatory = $true)]
  [string]$BuiltTrayPath
)

$ErrorActionPreference = "Stop"

$prefix = "scryer-windows-$Architecture"
$defenderLog = "$prefix-defender-scan.log"
$attachmentLog = "$prefix-attachment-services.log"
$startupLog = "$prefix-noarg-startup.log"
$wingetLog = "$prefix-winget-install.log"
$msiLog = "$prefix-msi-install.log"
$validationRoot = Join-Path $env:RUNNER_TEMP "scryer-package-validation-$Architecture"

function Write-Log {
  param(
    [Parameter(Mandatory = $true)]
    [string]$Path,

    [Parameter(Mandatory = $true)]
    [string]$Message
  )

  $line = "$(Get-Date -Format o) $Message"
  Write-Host $line
  Add-Content -Path $Path -Value $line
}

function Reset-NativeExitCode {
  $global:LASTEXITCODE = 0
}

function Restore-EnvVar {
  param(
    [Parameter(Mandatory = $true)]
    [string]$Name,

    [AllowNull()]
    [string]$Value
  )

  if ($null -eq $Value) {
    Remove-Item -Path "Env:$Name" -ErrorAction SilentlyContinue
  } else {
    Set-Item -Path "Env:$Name" -Value $Value
  }
}

function Get-MpCmdRun {
  $candidates = @()
  if (${env:ProgramFiles}) {
    $candidates += Join-Path ${env:ProgramFiles} "Windows Defender\MpCmdRun.exe"
  }

  $platformRoot = Join-Path $env:ProgramData "Microsoft\Windows Defender\Platform"
  if (Test-Path $platformRoot) {
    $candidates += Get-ChildItem $platformRoot -Recurse -Filter MpCmdRun.exe -ErrorAction SilentlyContinue |
      Sort-Object FullName -Descending |
      Select-Object -ExpandProperty FullName
  }

  foreach ($candidate in $candidates) {
    if ($candidate -and (Test-Path $candidate)) {
      return $candidate
    }
  }

  return $null
}

function Invoke-DefenderScan {
  param(
    [Parameter(Mandatory = $true)]
    [string]$Path
  )

  $mp = Get-MpCmdRun
  if (-not $mp) {
    Write-Log $defenderLog "MpCmdRun.exe was not found; Defender scan skipped."
    return
  }

  Write-Log $defenderLog "Scanning $Path with $mp"
  & $mp -Scan -ScanType 3 -File $Path -DisableRemediation *>> $defenderLog
  if ($LASTEXITCODE -ne 0) {
    throw "Defender scan failed for $Path with exit code $LASTEXITCODE. See $defenderLog."
  }
}

function Invoke-AttachmentServicesSave {
  param(
    [Parameter(Mandatory = $true)]
    [string]$Path,

    [Parameter(Mandatory = $true)]
    [string]$Source
  )

  Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;

[ComImport]
[Guid("4125dd96-e03a-4103-8f70-e0597d803b9c")]
public class AttachmentServices
{
}

[ComImport]
[Guid("73db1241-1e85-4581-8e4f-a81e1d0f8c57")]
[InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IAttachmentExecute
{
    void SetClientTitle([MarshalAs(UnmanagedType.LPWStr)] string clientTitle);
    void SetClientGuid(ref Guid guid);
    void SetLocalPath([MarshalAs(UnmanagedType.LPWStr)] string localPath);
    void SetFileName([MarshalAs(UnmanagedType.LPWStr)] string fileName);
    void SetSource([MarshalAs(UnmanagedType.LPWStr)] string source);
    void SetReferrer([MarshalAs(UnmanagedType.LPWStr)] string referrer);
    [PreserveSig] int CheckPolicy();
    [PreserveSig] int Prompt(IntPtr parent, int prompt, out int action);
    [PreserveSig] int Save();
    [PreserveSig] int Execute(IntPtr parent, [MarshalAs(UnmanagedType.LPWStr)] string verb, IntPtr processHandle);
    [PreserveSig] int SaveWithUI(IntPtr parent);
    void ClearClientState();
}

public static class ScryerAttachmentValidation
{
    public static int Save(string localPath, string source)
    {
        IAttachmentExecute attachment = (IAttachmentExecute)new AttachmentServices();
        attachment.SetLocalPath(localPath);
        attachment.SetFileName(System.IO.Path.GetFileName(localPath));
        attachment.SetSource(source);
        return attachment.Save();
    }
}
"@

  Write-Log $attachmentLog "Calling IAttachmentExecute::Save for $Path from $Source"
  $hr = [ScryerAttachmentValidation]::Save($Path, $Source)
  $unsigned = [uint32]$hr
  Write-Log $attachmentLog ("IAttachmentExecute::Save HRESULT: 0x{0:X8}" -f $unsigned)
  if ($hr -ne 0) {
    throw "Attachment Services rejected $Path with HRESULT 0x$($unsigned.ToString("X8")). See $attachmentLog."
  }
}

function Invoke-ScryerStartupSmoke {
  param(
    [Parameter(Mandatory = $true)]
    [string]$ExePath,

    [Parameter(Mandatory = $true)]
    [string]$Label
  )

  $startupRoot = Join-Path $validationRoot $Label
  $workDir = Join-Path $startupRoot "cwd"
  $localAppData = Join-Path $startupRoot "local-app-data"
  $appData = Join-Path $startupRoot "roaming-app-data"
  New-Item -ItemType Directory -Force -Path $workDir, $localAppData, $appData | Out-Null

  $oldLocalAppData = $env:LOCALAPPDATA
  $oldAppData = $env:APPDATA
  $oldBind = $env:SCRYER_BIND
  $oldOpenBrowser = $env:SCRYER_OPEN_BROWSER
  $oldAuthEnabled = $env:SCRYER_AUTH_ENABLED
  $process = $null

  try {
    $env:LOCALAPPDATA = $localAppData
    $env:APPDATA = $appData
    $env:SCRYER_OPEN_BROWSER = "false"
    $env:SCRYER_AUTH_ENABLED = "false"

    $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Parse("127.0.0.1"), 0)
    $listener.Start()
    try {
      $port = ([System.Net.IPEndPoint]$listener.LocalEndpoint).Port
    } finally {
      $listener.Stop()
    }
    $env:SCRYER_BIND = "127.0.0.1:$port"

    Write-Log $startupLog "Starting $ExePath with no arguments from $workDir"
    $process = Start-Process -FilePath $ExePath -WorkingDirectory $workDir -PassThru
    $session = New-Object Microsoft.PowerShell.Commands.WebRequestSession
    $baseUrl = "http://127.0.0.1:$port"
    $deadline = (Get-Date).AddSeconds(45)
    $lastError = $null
    $ready = $false

    while ((Get-Date) -lt $deadline) {
      if ($process.HasExited) {
        throw "Scryer exited before API readiness with code $($process.ExitCode)"
      }

      try {
        $spa = Invoke-WebRequest -Uri "$baseUrl/" -WebSession $session -TimeoutSec 5
        if ($spa.StatusCode -ne 200) {
          throw "SPA returned HTTP $($spa.StatusCode)"
        }

        $proofResponse = Invoke-WebRequest -Uri "$baseUrl/authless-client" -Headers @{ Accept = "application/json" } -WebSession $session -TimeoutSec 5
        if ($proofResponse.StatusCode -ne 200) {
          throw "Authless web client proof returned HTTP $($proofResponse.StatusCode)"
        }
        $proofText = if ($proofResponse.Content -is [byte[]]) {
          [System.Text.Encoding]::UTF8.GetString($proofResponse.Content)
        } else {
          [string]$proofResponse.Content
        }
        $proof = ($proofText | ConvertFrom-Json).proof
        if ([string]::IsNullOrWhiteSpace($proof)) {
          throw "Authless web client proof response did not include a proof"
        }

        $payload = @{ query = "query { scryerVersion }" } | ConvertTo-Json -Compress
        $api = Invoke-WebRequest -Uri "$baseUrl/graphql" -Method Post -ContentType "application/json" -Body $payload -Headers @{ "X-Scryer-Web-Client" = $proof } -WebSession $session -TimeoutSec 5
        $responseText = if ($api.Content -is [byte[]]) {
          [System.Text.Encoding]::UTF8.GetString($api.Content)
        } else {
          [string]$api.Content
        }
        $json = $responseText | ConvertFrom-Json
        if ($json.errors) {
          throw "GraphQL errors: $($json.errors | ConvertTo-Json -Compress -Depth 8)"
        }
        if ([string]::IsNullOrWhiteSpace($json.data.scryerVersion)) {
          throw "GraphQL scryerVersion did not include a version"
        }

        Write-Log $startupLog "$Label startup API smoke passed with version $($json.data.scryerVersion)"
        $ready = $true
        break
      } catch {
        $lastError = $_.Exception.Message
        Start-Sleep -Milliseconds 500
      }
    }

    if (-not $ready) {
      throw "Timed out waiting for Scryer API readiness. Last error: $lastError"
    }
  } finally {
    if ($process -and -not $process.HasExited) {
      Stop-Process -Id $process.Id -ErrorAction SilentlyContinue
      try {
        Wait-Process -Id $process.Id -Timeout 10 -ErrorAction Stop
      } catch {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
      }
    }

    $defaultLog = Join-Path $localAppData "scryer\logs\scryer.log"
    if (Test-Path $defaultLog) {
      Add-Content -Path $startupLog -Value "----- Scryer log tail -----"
      Get-Content $defaultLog -Tail 160 | Add-Content -Path $startupLog
      Add-Content -Path $startupLog -Value "----- end Scryer log tail -----"
    } else {
      Write-Log $startupLog "Default Scryer log was not created at $defaultLog"
    }

    Restore-EnvVar -Name "LOCALAPPDATA" -Value $oldLocalAppData
    Restore-EnvVar -Name "APPDATA" -Value $oldAppData
    Restore-EnvVar -Name "SCRYER_BIND" -Value $oldBind
    Restore-EnvVar -Name "SCRYER_OPEN_BROWSER" -Value $oldOpenBrowser
    Restore-EnvVar -Name "SCRYER_AUTH_ENABLED" -Value $oldAuthEnabled
  }
}

function Get-ProgramFiles64 {
  if ($env:ProgramW6432) {
    return $env:ProgramW6432
  }

  return ${env:ProgramFiles}
}

function Invoke-MsiExec {
  param(
    [Parameter(Mandatory = $true)]
    [string[]]$Arguments
  )

  # Scryer has no 32-bit package. Sysnative ensures a mistakenly launched
  # 32-bit PowerShell host still invokes the native Windows Installer.
  $directory = if ([Environment]::Is64BitProcess) { "System32" } else { "Sysnative" }
  $msiExec = Join-Path $env:WINDIR "$directory\msiexec.exe"
  return (Start-Process -FilePath $msiExec -ArgumentList $Arguments -PassThru -Wait).ExitCode
}

function Get-MsiRegistryStringValue {
  param(
    [Parameter(Mandatory = $true)]
    [string]$MsiPath,

    [Parameter(Mandatory = $true)]
    [string]$Key,

    [Parameter(Mandatory = $true)]
    [string]$Name
  )

  $escapedKey = $Key.Replace("'", "''")
  $escapedName = $Name.Replace("'", "''")
  $installer = New-Object -ComObject WindowsInstaller.Installer
  $database = $installer.OpenDatabase($MsiPath, 0)
  $query = 'SELECT `Value` FROM `Registry` WHERE `Root`=2 AND `Key`=''{0}'' AND `Name`=''{1}''' -f $escapedKey, $escapedName
  $view = $database.OpenView($query)
  $view.Execute()
  try {
    $record = $view.Fetch()
    if (-not $record) {
      throw "MSI did not contain HKLM\\${Key}::$Name in its Registry table: $MsiPath"
    }

    # Convert the COM field to a plain string while its view remains open. Raw
    # Windows Installer record values are not reliable after their view closes.
    $value = $record.StringData(1)
    if ($null -eq $value) {
      throw "MSI registry row HKLM\\${Key}::$Name had no value: $MsiPath"
    }

    return $value.ToString()
  } finally {
    [void]$view.Close()
  }
}

function Assert-MsiDistributionOwner {
  param(
    [Parameter(Mandatory = $true)]
    [string]$MsiPath,

    [Parameter(Mandatory = $true)]
    [string]$ExpectedOwner
  )

  $actualOwner = (Get-MsiRegistryStringValue `
    -MsiPath $MsiPath `
    -Key "Software\Scryer Media\Scryer" `
    -Name "DistributionOwner").Trim()
  if ($actualOwner -ne $ExpectedOwner) {
    throw "MSI DistributionOwner was '$actualOwner', expected '$ExpectedOwner': $MsiPath"
  }
}

function Invoke-WinGetManifestValidation {
  param(
    [Parameter(Mandatory = $true)]
    [string]$PackageMsi,

    [Parameter(Mandatory = $true)]
    [string]$ProductCode,

    [Parameter(Mandatory = $true)]
    [string]$PackageVersion
  )

  $winget = (Get-Command winget.exe -ErrorAction SilentlyContinue).Source
  if (-not $winget) {
    try {
      Add-AppxPackage -RegisterByFamilyName -MainPackage Microsoft.DesktopAppInstaller_8wekyb3d8bbwe -ErrorAction Stop
    } catch {
      throw "winget.exe was not found and the App Installer could not be registered: $($_.Exception.Message)"
    }

    $appInstaller = Get-AppxPackage -Name Microsoft.DesktopAppInstaller -ErrorAction SilentlyContinue |
      Select-Object -First 1
    if ($appInstaller) {
      $candidate = Join-Path $appInstaller.InstallLocation "winget.exe"
      if (Test-Path -LiteralPath $candidate) {
        $winget = $candidate
      }
    }

    if (-not $winget) {
      $winget = (Get-Command winget.exe -ErrorAction SilentlyContinue).Source
    }
    if (-not $winget) {
      throw "winget.exe was not found after registering the App Installer; MSI manifest validation is required."
    }
  }

  $manifestRoot = Join-Path $validationRoot "winget-manifest"
  New-Item -ItemType Directory -Force -Path $manifestRoot | Out-Null
  $msiHash = (Get-FileHash $PackageMsi -Algorithm SHA256).Hash.ToUpperInvariant()
  $wingetArchitecture = if ($Architecture -eq "x86_64") { "x64" } else { "arm64" }

  @"
# yaml-language-server: `$schema=https://aka.ms/winget-manifest.version.1.10.0.schema.json
PackageIdentifier: ScryerMedia.Scryer
PackageVersion: $PackageVersion
DefaultLocale: en-US
ManifestType: version
ManifestVersion: 1.10.0
"@ | Set-Content -Path (Join-Path $manifestRoot "ScryerMedia.Scryer.yaml") -Encoding utf8

  @"
# yaml-language-server: `$schema=https://aka.ms/winget-manifest.defaultLocale.1.10.0.schema.json
PackageIdentifier: ScryerMedia.Scryer
PackageVersion: $PackageVersion
PackageLocale: en-US
Publisher: Scryer Media
PackageName: Scryer
License: GPL-3.0
ShortDescription: Self-hosted media acquisition and management platform.
ManifestType: defaultLocale
ManifestVersion: 1.10.0
"@ | Set-Content -Path (Join-Path $manifestRoot "ScryerMedia.Scryer.locale.en-US.yaml") -Encoding utf8

  @"
# yaml-language-server: `$schema=https://aka.ms/winget-manifest.installer.1.10.0.schema.json
PackageIdentifier: ScryerMedia.Scryer
PackageVersion: $PackageVersion
InstallerType: msi
UpgradeBehavior: uninstallPrevious
Installers:
- Architecture: $wingetArchitecture
  InstallerUrl: https://github.com/scryer-media/scryer/releases/download/scryer-local-ci/$(Split-Path $PackageMsi -Leaf)
  InstallerSha256: $msiHash
  ProductCode: '$ProductCode'
ManifestType: installer
ManifestVersion: 1.10.0
"@ | Set-Content -Path (Join-Path $manifestRoot "ScryerMedia.Scryer.installer.yaml") -Encoding utf8

  $expectedSchemaHeaders = @{
    "ScryerMedia.Scryer.yaml" = '# yaml-language-server: $schema=https://aka.ms/winget-manifest.version.1.10.0.schema.json'
    "ScryerMedia.Scryer.locale.en-US.yaml" = '# yaml-language-server: $schema=https://aka.ms/winget-manifest.defaultLocale.1.10.0.schema.json'
    "ScryerMedia.Scryer.installer.yaml" = '# yaml-language-server: $schema=https://aka.ms/winget-manifest.installer.1.10.0.schema.json'
  }
  foreach ($manifest in $expectedSchemaHeaders.GetEnumerator()) {
    $manifestPath = Join-Path $manifestRoot $manifest.Key
    $actualHeader = Get-Content -LiteralPath $manifestPath -TotalCount 1
    if ($actualHeader -ne $manifest.Value) {
      throw "Generated WinGet manifest '$($manifest.Key)' has an invalid schema header: '$actualHeader'"
    }
  }

  Write-Log $wingetLog "Validating generated MSI winget manifest from $manifestRoot"
  & $winget validate --manifest $manifestRoot --disable-interactivity *>> $wingetLog
  $manifestValidationExitCode = $LASTEXITCODE
  if ($manifestValidationExitCode -eq -1978335192) {
    Write-Log $wingetLog "WinGet manifest validation succeeded with warnings; schema headers were checked explicitly."
  } elseif ($manifestValidationExitCode -ne 0) {
    Get-Content -LiteralPath $wingetLog -ErrorAction SilentlyContinue | ForEach-Object { Write-Host $_ }
    throw "winget manifest validation exited with code $manifestValidationExitCode"
  }
}

Remove-Item -Recurse -Force $validationRoot -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $validationRoot | Out-Null
"" | Set-Content $defenderLog
"" | Set-Content $attachmentLog
"" | Set-Content $startupLog
"" | Set-Content $wingetLog
"" | Set-Content $msiLog

$zipCopy = Join-Path $validationRoot (Split-Path $ZipPath -Leaf)
$msiCopy = Join-Path $validationRoot (Split-Path $MsiPath -Leaf)
$wingetMsiCopy = Join-Path $validationRoot (Split-Path $WingetMsiPath -Leaf)
$extractRoot = Join-Path $validationRoot "extracted"
Copy-Item $ZipPath $zipCopy -Force
Copy-Item $MsiPath $msiCopy -Force
Copy-Item $WingetMsiPath $wingetMsiCopy -Force
Expand-Archive -Path $zipCopy -DestinationPath $extractRoot -Force
$packagedExe = Join-Path $extractRoot "scryer.exe"
$packagedTray = Join-Path $extractRoot "scryer-tray.exe"
if (-not (Test-Path $packagedExe)) {
  throw "Packaged zip did not contain scryer.exe at the zip root."
}
if (-not (Test-Path $packagedTray)) {
  throw "Packaged zip did not contain scryer-tray.exe at the zip root."
}

$builtHash = (Get-FileHash $BuiltExePath -Algorithm SHA256).Hash
$packagedHash = (Get-FileHash $packagedExe -Algorithm SHA256).Hash
if ($builtHash -ne $packagedHash) {
  throw "Packaged scryer.exe hash differs from built executable."
}
$builtTrayHash = (Get-FileHash $BuiltTrayPath -Algorithm SHA256).Hash
$packagedTrayHash = (Get-FileHash $packagedTray -Algorithm SHA256).Hash
if ($builtTrayHash -ne $packagedTrayHash) {
  throw "Packaged scryer-tray.exe hash differs from built executable."
}

# The tarball is the artifact the in-app upgrade engine downloads and extracts,
# so its member names must be exactly the flat names the signed manifest records
# and the helper swap resolves against the staged directory.
$tarballCopy = Join-Path $validationRoot (Split-Path $TarballPath -Leaf)
$tarballExtractRoot = Join-Path $validationRoot "extracted-tarball"
Copy-Item $TarballPath $tarballCopy -Force
New-Item -ItemType Directory -Force -Path $tarballExtractRoot | Out-Null
tar --extract --gzip --file $tarballCopy --directory $tarballExtractRoot
if ($LASTEXITCODE -ne 0) {
  throw "Failed to extract the portable upgrade tarball."
}
$tarballMembers = Get-ChildItem -Recurse -File $tarballExtractRoot |
  ForEach-Object { $_.FullName.Substring($tarballExtractRoot.Length + 1).Replace('\', '/') } |
  Sort-Object
$expectedMembers = @("LICENSE", "README.txt", "scryer-tray.exe", "scryer.exe")
if (Compare-Object $tarballMembers $expectedMembers) {
  throw "Portable tarball members were '$($tarballMembers -join ", ")', expected '$($expectedMembers -join ", ")'."
}
$tarballExe = Join-Path $tarballExtractRoot "scryer.exe"
$tarballTray = Join-Path $tarballExtractRoot "scryer-tray.exe"
if ($builtHash -ne (Get-FileHash $tarballExe -Algorithm SHA256).Hash) {
  throw "Tarballed scryer.exe hash differs from built executable."
}
if ($builtTrayHash -ne (Get-FileHash $tarballTray -Algorithm SHA256).Hash) {
  throw "Tarballed scryer-tray.exe hash differs from built executable."
}

$msiMetadata = Get-Content $MsiMetadataPath -Raw | ConvertFrom-Json
$wingetMsiMetadata = Get-Content $WingetMsiMetadataPath -Raw | ConvertFrom-Json
foreach ($metadata in @($msiMetadata, $wingetMsiMetadata)) {
  if ($metadata.product_code -notmatch '^\{[0-9A-Fa-f]{8}(-[0-9A-Fa-f]{4}){3}-[0-9A-Fa-f]{12}\}$') {
    throw "MSI metadata did not contain a valid ProductCode: $($metadata.product_code)"
  }
  if ($metadata.version -notmatch '^\d+\.\d+\.\d+$') {
    throw "MSI metadata did not contain a valid release version: $($metadata.version)"
  }
}
if ($msiMetadata.distribution_owner -ne "msi") {
  throw "Primary MSI metadata DistributionOwner was '$($msiMetadata.distribution_owner)', expected 'msi'"
}
if ($wingetMsiMetadata.distribution_owner -ne "winget") {
  throw "WinGet MSI metadata DistributionOwner was '$($wingetMsiMetadata.distribution_owner)', expected 'winget'"
}
if ($msiMetadata.upgrade_code -ne $wingetMsiMetadata.upgrade_code) {
  throw "MSI variants did not share an UpgradeCode."
}
if ($msiMetadata.version -ne $wingetMsiMetadata.version) {
  throw "MSI variants did not share a version."
}
if ($msiMetadata.architecture -ne $wingetMsiMetadata.architecture) {
  throw "MSI variants did not share an architecture."
}
if ($msiMetadata.product_code -eq $wingetMsiMetadata.product_code) {
  throw "MSI variants must have distinct ProductCodes."
}
Assert-MsiDistributionOwner -MsiPath $MsiPath -ExpectedOwner "msi"
Assert-MsiDistributionOwner -MsiPath $WingetMsiPath -ExpectedOwner "winget"

Invoke-DefenderScan -Path $zipCopy
Invoke-DefenderScan -Path $tarballCopy
Invoke-DefenderScan -Path $packagedExe
Invoke-DefenderScan -Path $packagedTray
Invoke-DefenderScan -Path $msiCopy
Invoke-DefenderScan -Path $wingetMsiCopy

$sourceUrl = "https://github.com/scryer-media/scryer/releases/download/scryer-local-ci/$(Split-Path $zipCopy -Leaf)"
Invoke-AttachmentServicesSave -Path $zipCopy -Source $sourceUrl
Invoke-AttachmentServicesSave -Path $msiCopy -Source ($sourceUrl -replace 'zip$', 'msi')
Invoke-AttachmentServicesSave -Path $wingetMsiCopy -Source "https://github.com/scryer-media/scryer/releases/download/scryer-local-ci/$(Split-Path $wingetMsiCopy -Leaf)"

Invoke-ScryerStartupSmoke -ExePath $packagedExe -Label "packaged"

foreach ($unsignedArtifact in @($packagedExe, $packagedTray, $msiCopy, $wingetMsiCopy)) {
  $signature = Get-AuthenticodeSignature -FilePath $unsignedArtifact
  if ($signature.Status -ne "NotSigned") {
    throw "Expected intentionally unsigned artifact $unsignedArtifact, got Authenticode status $($signature.Status)."
  }
}

$desktopProfile = Join-Path $env:LOCALAPPDATA "ScryerMedia\Scryer"
$profileMarker = Join-Path $desktopProfile "preserve-on-uninstall.txt"
New-Item -ItemType Directory -Force -Path $desktopProfile | Out-Null
"preserve me" | Set-Content $profileMarker

$msiExitCode = Invoke-MsiExec -Arguments @("/i", $msiCopy, "/qn", "/norestart", "/l*v", $msiLog)
if ($msiExitCode -ne 0) {
  throw "MSI install failed with exit code $msiExitCode. See $msiLog."
}

$installDir = Join-Path (Get-ProgramFiles64) "Scryer Media\Scryer"
$installedExe = Join-Path $installDir "scryer.exe"
$installedTray = Join-Path $installDir "scryer-tray.exe"
foreach ($required in @($installedExe, $installedTray, (Join-Path $installDir "LICENSE"))) {
  if (-not (Test-Path $required)) {
    throw "MSI did not install expected payload file $required"
  }
}
if ((Get-FileHash $installedExe -Algorithm SHA256).Hash -ne $builtHash) {
  throw "MSI-installed scryer.exe hash differs from the built executable."
}
if ((Get-FileHash $installedTray -Algorithm SHA256).Hash -ne $builtTrayHash) {
  throw "MSI-installed scryer-tray.exe hash differs from the built executable."
}
if (-not (Test-Path (Join-Path $env:ProgramData "Microsoft\Windows\Start Menu\Programs\Scryer\Scryer.lnk"))) {
  throw "MSI did not create the Scryer Start Menu shortcut."
}
if (Get-Process scryer-tray -ErrorAction SilentlyContinue) {
  throw "Silent MSI install started scryer-tray.exe; silent installs must stay quiet."
}
if (([Environment]::GetEnvironmentVariable("Path", "Machine")) -match [regex]::Escape($installDir)) {
  throw "MSI added its install directory to the machine PATH."
}
if (Get-CimInstance Win32_Service | Where-Object { $_.PathName -match [regex]::Escape($installDir) }) {
  throw "MSI registered a Windows service for Scryer."
}
& $installedExe --version *>> $msiLog
if ($LASTEXITCODE -ne 0) {
  throw "MSI-installed scryer.exe --version failed with exit code $LASTEXITCODE."
}

$msiExitCode = Invoke-MsiExec -Arguments @("/fa", $msiCopy, "/qn", "/norestart", "/l*v", $msiLog)
if ($msiExitCode -ne 0) {
  throw "MSI repair failed with exit code $msiExitCode. See $msiLog."
}
$msiExitCode = Invoke-MsiExec -Arguments @("/x", $msiMetadata.product_code, "/qn", "/norestart", "/l*v", $msiLog)
if ($msiExitCode -ne 0) {
  throw "MSI uninstall failed with exit code $msiExitCode. See $msiLog."
}
if (Test-Path $installDir) {
  throw "MSI uninstall retained the Program Files payload directory $installDir."
}
if (-not (Test-Path $profileMarker)) {
  throw "MSI uninstall removed desktop user data at $profileMarker."
}

Invoke-WinGetManifestValidation `
  -PackageMsi $wingetMsiCopy `
  -ProductCode $wingetMsiMetadata.product_code `
  -PackageVersion $wingetMsiMetadata.version
