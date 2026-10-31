$ErrorActionPreference = "Stop"
Import-Module Microsoft.PowerShell.Utility
. "$PSScriptRoot/windows-startup-auth.ps1"

function Assert-That($Condition, [string]$Message) {
  if (-not $Condition) { throw $Message }
}

# Replace only the HTTP boundary; exercise the real setup and proof handling.
function Invoke-WebRequest {
  param($Uri, $Headers, $WebSession, $TimeoutSec, [switch]$SkipHttpErrorCheck)
  Assert-That ($Uri -eq "http://127.0.0.1:18080/authless-client") "Unexpected proof URL"
  Assert-That ($WebSession -eq $script:session) "Proof request lost its cookie session"
  Assert-That $SkipHttpErrorCheck "Setup refusals must be classified by their reason code"
  return $script:responses.Dequeue()
}

function Invoke-RestMethod {
  param($Uri, $Method, $ContentType, $Body, $WebSession, $TimeoutSec)
  Assert-That ($Uri -eq "http://127.0.0.1:18080/graphql") "Unexpected setup URL"
  Assert-That ($Method -eq "Post") "Setup must use POST"
  Assert-That ($ContentType -eq "application/json") "Setup must use JSON"
  Assert-That ($WebSession -eq $script:session) "Setup request lost its cookie session"
  $script:mutations.Add(($Body | ConvertFrom-Json))
  return $script:setupResult
}

function Test-ProofCase {
  param([object[]]$Responses, [bool]$Fails, [int]$MutationCount, $SetupResult)
  $script:responses = [System.Collections.Generic.Queue[object]]::new()
  foreach ($response in $Responses) { $script:responses.Enqueue($response) }
  $script:mutations = [System.Collections.Generic.List[object]]::new()
  $script:setupResult = $SetupResult
  $failed = $false
  try {
    $proof = Get-ScryerStartupProof -BaseUrl "http://127.0.0.1:18080" -Session $session
  } catch {
    $failed = $true
  }
  Assert-That ($failed -eq $Fails) "Unexpected success/failure outcome"
  Assert-That ($script:mutations.Count -eq $MutationCount) "Unexpected setup mutation count"
  if (-not $Fails) { Assert-That ($proof -eq "browser-proof") "Expected the issued browser proof" }
}

$session = [Microsoft.PowerShell.Commands.WebRequestSession]::new()
$challenge = @{ StatusCode = 403; Content = '{"errors":[{"extensions":{"reason":"INITIAL_AUTH_REQUIRED"}}]}' }
$issued = @{ StatusCode = 200; Content = '{"proof":"browser-proof"}' }
$completed = @{ data = @{ completeInitialAuthentication = @{ completed = $true } } }
Test-ProofCase -Responses @($issued) -Fails $false -MutationCount 0
Test-ProofCase -Responses @($challenge, $issued) -Fails $false -MutationCount 1 -SetupResult $completed
$input = $script:mutations[0].variables.input
Assert-That ($input.mode -eq "NO_AUTHENTICATION") "Expected explicit no-authentication selection"
Assert-That ($input.acknowledgeNoAuthentication -eq $true) "Expected acknowledgement"
Assert-That ($input.PSObject.Properties.Name -notcontains "setupCode") "Expected no setup code"

Test-ProofCase -Responses @(@{ StatusCode = 403; Content = '{"errors":[{"extensions":{"reason":"OTHER_REFUSAL"}}]}' }) -Fails $true -MutationCount 0
Test-ProofCase -Responses @($challenge) -Fails $true -MutationCount 1 -SetupResult @{ errors = @(@{ extensions = @{ reason = "INITIAL_AUTH_MODE_CONFLICT" } }) }
Test-ProofCase -Responses @($challenge) -Fails $true -MutationCount 1 -SetupResult @{ data = @{ completeInitialAuthentication = @{ completed = $false } } }
Test-ProofCase -Responses @(@{ StatusCode = 200; Content = '{}' }) -Fails $true -MutationCount 0
Test-ProofCase -Responses @(@{ StatusCode = 503; Content = '{}' }) -Fails $true -MutationCount 0
Test-ProofCase -Responses @(@{ StatusCode = 200; Content = [System.Text.Encoding]::UTF8.GetBytes('{"proof":"browser-proof"}') }) -Fails $false -MutationCount 0
Write-Host "Windows startup authentication: 8 cases passed"
