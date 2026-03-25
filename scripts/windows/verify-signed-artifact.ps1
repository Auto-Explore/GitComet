param(
  [Parameter(Mandatory = $true)]
  [string]$Path,

  [Parameter(Mandatory = $true)]
  [string]$SignToolPath
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if (!(Test-Path -LiteralPath $Path)) {
  throw "Expected signed artifact '$Path' to exist."
}

$signature = Get-AuthenticodeSignature -FilePath $Path
if ($signature.Status -ne [System.Management.Automation.SignatureStatus]::Valid) {
  throw "Authenticode verification failed for '$Path': $($signature.Status) $($signature.StatusMessage)"
}
if ($null -eq $signature.SignerCertificate) {
  throw "Missing signer certificate on '$Path'."
}
if ($null -eq $signature.TimeStamperCertificate) {
  throw "Missing RFC3161 timestamp on '$Path'."
}

$verifyOutput = & $SignToolPath verify /pa /all /v /tw $Path 2>&1
$verifyOutput | ForEach-Object { Write-Host $_ }
if ($LASTEXITCODE -ne 0) {
  throw "signtool verification failed for '$Path'."
}
if (($verifyOutput | Out-String) -match 'No timestamp') {
  throw "signtool reported a missing timestamp for '$Path'."
}
