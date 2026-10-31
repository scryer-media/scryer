# Complete the fresh-install decision the same way the UI does. Startup smoke
# runs never set SCRYER_REQUIRE_SETUP_CODE, so the choice carries no setup code.
function Get-ScryerStartupProof {
  param(
    [string]$BaseUrl,
    [Microsoft.PowerShell.Commands.WebRequestSession]$Session
  )

  $request = @{
    Uri = "$BaseUrl/authless-client"
    Headers = @{ Accept = "application/json" }
    WebSession = $Session
    TimeoutSec = 5
    SkipHttpErrorCheck = $true
  }
  $response = Invoke-WebRequest @request
  if ($response.StatusCode -eq 403) {
    $refusal = $response.Content | ConvertFrom-Json
    if ($refusal.errors.extensions.reason -notcontains "INITIAL_AUTH_REQUIRED") {
      throw "Authless web client proof was refused without an initial-setup challenge"
    }

    $payload = @{
      query = 'mutation($input: CompleteInitialAuthenticationInput!) { completeInitialAuthentication(input: $input) { completed } }'
      variables = @{ input = @{
        mode = "NO_AUTHENTICATION"
        acknowledgeNoAuthentication = $true
      } }
    } | ConvertTo-Json -Depth 5 -Compress
    $setup = Invoke-RestMethod -Uri "$BaseUrl/graphql" -Method Post -ContentType "application/json" -Body $payload -WebSession $Session -TimeoutSec 5
    if ($setup.errors -or $setup.data.completeInitialAuthentication.completed -ne $true) {
      throw "Initial authentication decision did not complete"
    }
    $response = Invoke-WebRequest @request
  }

  if ($response.StatusCode -ne 200) {
    throw "Authless web client proof returned HTTP $($response.StatusCode)"
  }
  $content = if ($response.Content -is [byte[]]) {
    [System.Text.Encoding]::UTF8.GetString($response.Content)
  } else {
    [string]$response.Content
  }
  $proof = ($content | ConvertFrom-Json).proof
  if ([string]::IsNullOrWhiteSpace($proof)) {
    throw "Authless web client proof response did not include a proof"
  }
  return $proof
}
