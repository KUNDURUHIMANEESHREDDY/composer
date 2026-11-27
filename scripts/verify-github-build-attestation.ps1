# Verify a real GitHub Sigstore attestation and its exact release identity.
# Editable predicate metadata alone must never be accepted as signer identity.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$ArtifactPath,
    [Parameter(Mandatory = $true)][string]$WorkflowFile,
    [Parameter(Mandatory = $true)][string]$Repository,
    [Parameter(Mandatory = $true)][string]$Commit,
    [Parameter(Mandatory = $true)][string]$SourceRef,
    [Parameter(Mandatory = $true)][string]$RunId,
    [Parameter(Mandatory = $true)][string]$RunAttempt
)
$ErrorActionPreference = 'Stop'
if ($Repository -cne 'KUNDURUHIMANEESHREDDY/composer') {
    throw 'Unexpected attestation repository'
}
if ($WorkflowFile -cnotin @('multisource-attested-cli.yml', 'second-target-installer.yml')) {
    throw 'Unapproved artifact signer workflow'
}
if ($Commit -cnotmatch '^[0-9a-f]{40}$') { throw 'Expected commit must be a full lowercase Git SHA' }
if ($SourceRef -notmatch '^refs/(heads/[A-Za-z0-9_./-]+|pull/[0-9]+/merge)$') {
    throw 'Unexpected source reference'
}
if ($RunId -notmatch '^[1-9][0-9]*$' -or $RunAttempt -notmatch '^[1-9][0-9]*$') {
    throw 'Run ID and run attempt are required'
}
if (-not (Test-Path -LiteralPath $ArtifactPath -PathType Leaf)) {
    throw 'Expected artifact is absent'
}
$ArtifactPath = (Resolve-Path -LiteralPath $ArtifactPath).Path
$actualDigest = (Get-FileHash -LiteralPath $ArtifactPath -Algorithm SHA256).Hash.ToLowerInvariant()
$workflowIdentity = "$Repository/.github/workflows/$WorkflowFile"
$expectedSigner = "https://github.com/$workflowIdentity@$SourceRef"
$expectedRun = "https://github.com/$Repository/actions/runs/$RunId/attempts/$RunAttempt"

# gh validates Sigstore signature, cert chain, GitHub issuer, SLSA type, and
# SHA-256 of the actual artifact. Flags enforce signer and source identity.
$raw = & gh attestation verify $ArtifactPath `
    --repo $Repository `
    --signer-workflow $workflowIdentity `
    --signer-digest $Commit `
    --source-digest $Commit `
    --source-ref $SourceRef `
    --deny-self-hosted-runners `
    --predicate-type 'https://slsa.dev/provenance/v1' `
    --format json
if ($LASTEXITCODE -ne 0) { throw 'GitHub attestation cryptographic or identity verification failed' }
if (-not $raw) { throw 'No verified attestation was returned' }
try {
    $records = @(($raw -join "`n" | ConvertFrom-Json))
} catch {
    throw 'GitHub attestation output was not valid JSON'
}
$valid = $false
foreach ($record in $records) {
    $result = $record.verificationResult
    if ($null -eq $result -or $null -eq $result.signature -or $null -eq $result.statement) {
        continue
    }
    $certificate = $result.signature.certificate
    if ($null -eq $certificate) { continue }
    if ($certificate.issuer -cne 'https://token.actions.githubusercontent.com') { continue }
    if ($certificate.sourceRepositoryURI -cne "https://github.com/$Repository") { continue }
    if ($certificate.githubWorkflowRepository -cne $Repository) { continue }
    if ($certificate.buildSignerURI -cne $expectedSigner) { continue }
    if ($certificate.sourceRepositoryDigest -cne $Commit) { continue }
    if ($certificate.buildSignerDigest -cne $Commit) { continue }
    if ($certificate.sourceRepositoryRef -cne $SourceRef) { continue }
    if ($certificate.runInvocationURI -cne $expectedRun) { continue }
    if ($result.statement.predicateType -cne 'https://slsa.dev/provenance/v1') { continue }
    foreach ($subject in @($result.statement.subject)) {
        if ($null -ne $subject -and
            $subject.name -ceq (Split-Path -Leaf $ArtifactPath) -and
            $subject.digest.sha256 -ceq $actualDigest) {
            $valid = $true
            break
        }
    }
    if ($valid) { break }
}
if (-not $valid) {
    throw 'Verified attestation did not match expected workflow, run, commit, ref and artifact digest'
}
Write-Output "PASS GitHub attestation identity and SHA256: $actualDigest"
