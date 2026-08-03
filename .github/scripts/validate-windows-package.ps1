param(
  [Parameter(Mandatory = $true)]
  [string]$Architecture,

  [Parameter(Mandatory = $true)]
  [string]$ZipPath,

  [Parameter(Mandatory = $true)]
  [string]$BuiltExePath
)

$ErrorActionPreference = "Stop"

$prefix = "scryer-windows-$Architecture"
$defenderLog = "$prefix-defender-scan.log"
$attachmentLog = "$prefix-attachment-services.log"
$startupLog = "$prefix-noarg-startup.log"
$wingetLog = "$prefix-winget-install.log"
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

        $payload = @{ query = "query { systemStatus { version } }" } | ConvertTo-Json -Compress
        $api = Invoke-WebRequest -Uri "$baseUrl/graphql" -Method Post -ContentType "application/json" -Body $payload -WebSession $session -TimeoutSec 5
        $responseText = if ($api.Content -is [byte[]]) {
          [System.Text.Encoding]::UTF8.GetString($api.Content)
        } else {
          [string]$api.Content
        }
        $json = $responseText | ConvertFrom-Json
        if ($json.errors) {
          throw "GraphQL errors: $($json.errors | ConvertTo-Json -Compress -Depth 8)"
        }
        if (-not $json.data.systemStatus.version) {
          throw "GraphQL systemStatus did not include a version"
        }

        Write-Log $startupLog "$Label startup API smoke passed with version $($json.data.systemStatus.version)"
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

function Invoke-WinGetLocalInstallSmoke {
  param(
    [Parameter(Mandatory = $true)]
    [string]$PackageZip
  )

  $winget = (Get-Command winget.exe -ErrorAction SilentlyContinue).Source
  if (-not $winget) {
    Write-Log $wingetLog "winget.exe was not found; local manifest install smoke skipped."
    return
  }

  $manifestRoot = Join-Path $validationRoot "winget-manifest"
  New-Item -ItemType Directory -Force -Path $manifestRoot | Out-Null
  $zipHash = (Get-FileHash $PackageZip -Algorithm SHA256).Hash.ToUpperInvariant()
  $zipUri = ([System.Uri](Resolve-Path $PackageZip).Path).AbsoluteUri
  $wingetArchitecture = if ($Architecture -eq "x86_64") { "x64" } else { "arm64" }
  $aliasName = "scryer-ci-$Architecture"

  @"
PackageIdentifier: ScryerMedia.Scryer
PackageVersion: 0.0.0
DefaultLocale: en-US
ManifestType: version
ManifestVersion: 1.12.0
"@ | Set-Content -Path (Join-Path $manifestRoot "ScryerMedia.Scryer.yaml") -Encoding utf8

  @"
PackageIdentifier: ScryerMedia.Scryer
PackageVersion: 0.0.0
PackageLocale: en-US
Publisher: Scryer Media
PackageName: Scryer
License: GPL-3.0
ShortDescription: Self-hosted media acquisition and management platform.
ManifestType: defaultLocale
ManifestVersion: 1.12.0
"@ | Set-Content -Path (Join-Path $manifestRoot "ScryerMedia.Scryer.locale.en-US.yaml") -Encoding utf8

  @"
PackageIdentifier: ScryerMedia.Scryer
PackageVersion: 0.0.0
InstallerType: zip
NestedInstallerType: portable
NestedInstallerFiles:
- RelativeFilePath: scryer.exe
  PortableCommandAlias: $aliasName
Installers:
- Architecture: $wingetArchitecture
  InstallerUrl: $zipUri
  InstallerSha256: $zipHash
ManifestType: installer
ManifestVersion: 1.12.0
"@ | Set-Content -Path (Join-Path $manifestRoot "ScryerMedia.Scryer.installer.yaml") -Encoding utf8

  Write-Log $wingetLog "Running winget local manifest install smoke from $manifestRoot"
  $installSucceeded = $false
  try {
    & $winget settings --enable LocalManifestFiles *>> $wingetLog
    & $winget install --manifest $manifestRoot --accept-package-agreements --accept-source-agreements --disable-interactivity *>> $wingetLog
    if ($LASTEXITCODE -ne 0) {
      throw "winget install exited with code $LASTEXITCODE"
    }
    $installSucceeded = $true
    Write-Log $wingetLog "winget local manifest install smoke succeeded."

    $installedExe = (Get-Command $aliasName -ErrorAction SilentlyContinue).Source
    if (-not $installedExe) {
      $installedExe = Get-ChildItem (Join-Path $env:LOCALAPPDATA "Microsoft\WinGet\Links") -Filter "$aliasName*" -ErrorAction SilentlyContinue |
        Select-Object -First 1 -ExpandProperty FullName
    }
    if (-not $installedExe -or -not (Test-Path $installedExe)) {
      throw "winget installed the package but did not create the $aliasName command alias"
    }

    & $installedExe --version *>> $wingetLog
    if ($LASTEXITCODE -ne 0) {
      throw "winget-installed Scryer --version exited with code $LASTEXITCODE"
    }
    Write-Log $wingetLog "winget-installed command alias executed successfully from $installedExe."
    Invoke-ScryerStartupSmoke -ExePath $installedExe -Label "winget-installed"
  } catch {
    if ($installSucceeded) {
      Write-Log $wingetLog "winget-installed Scryer validation failed: $($_.Exception.Message)"
      throw
    }
    Write-Log $wingetLog "winget local manifest install smoke was inconclusive: $($_.Exception.Message)"
    Write-Warning "winget local manifest install smoke was inconclusive; see $wingetLog."
  } finally {
    try {
      & $winget uninstall --id ScryerMedia.Scryer --accept-source-agreements --disable-interactivity *>> $wingetLog
    } catch {
      Write-Log $wingetLog "winget cleanup failed or package was not installed: $($_.Exception.Message)"
    } finally {
      # This smoke is evidence-only: the package archive, Defender, Attachment
      # Services, and startup checks above remain release-blocking.
      Reset-NativeExitCode
    }
  }
}

Remove-Item -Recurse -Force $validationRoot -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $validationRoot | Out-Null
"" | Set-Content $defenderLog
"" | Set-Content $attachmentLog
"" | Set-Content $startupLog
"" | Set-Content $wingetLog

$zipCopy = Join-Path $validationRoot (Split-Path $ZipPath -Leaf)
$extractRoot = Join-Path $validationRoot "extracted"
Copy-Item $ZipPath $zipCopy -Force
Expand-Archive -Path $zipCopy -DestinationPath $extractRoot -Force
$packagedExe = Join-Path $extractRoot "scryer.exe"
if (-not (Test-Path $packagedExe)) {
  throw "Packaged zip did not contain scryer.exe at the zip root."
}

$builtHash = (Get-FileHash $BuiltExePath -Algorithm SHA256).Hash
$packagedHash = (Get-FileHash $packagedExe -Algorithm SHA256).Hash
if ($builtHash -ne $packagedHash) {
  throw "Packaged scryer.exe hash differs from built executable."
}

Invoke-DefenderScan -Path $zipCopy
Invoke-DefenderScan -Path $packagedExe

$sourceUrl = "https://github.com/scryer-media/scryer/releases/download/scryer-local-ci/$(Split-Path $zipCopy -Leaf)"
Invoke-AttachmentServicesSave -Path $zipCopy -Source $sourceUrl

Invoke-ScryerStartupSmoke -ExePath $packagedExe -Label "packaged"
Invoke-WinGetLocalInstallSmoke -PackageZip $zipCopy
