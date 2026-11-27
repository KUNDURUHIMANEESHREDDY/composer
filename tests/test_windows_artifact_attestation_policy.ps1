# Synthetic negative controls only: real signed verification runs in Windows CI.
$ErrorActionPreference = 'Stop'
$verifier = Join-Path (Split-Path $PSScriptRoot -Parent) 'scripts/verify-github-build-attestation.ps1'
$dir = Join-Path ([IO.Path]::GetTempPath()) ('composer-provenance-test-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $dir | Out-Null
try {
    $artifact = Join-Path $dir 'composer-windows-x64.zip'
    [IO.File]::WriteAllText($artifact, 'synthetic test artifact')
    $digest = (Get-FileHash $artifact -Algorithm SHA256).Hash.ToLowerInvariant()
    $repository = 'KUNDURUHIMANEESHREDDY/composer'
    $commit = 'abcdef0123456789abcdef0123456789abcdef01'
    $ref = 'refs/pull/43/merge'
    $workflow = 'multisource-attested-cli.yml'
    $runId = '12345'
    $attempt = '2'
    $expectedSigner = "https://github.com/$repository/.github/workflows/$workflow@$ref"
    $expectedRun = "https://github.com/$repository/actions/runs/$runId/attempts/$attempt"
    $parameters = @{
        ArtifactPath = $artifact; Repository = $repository; WorkflowFile = $workflow
        Commit = $commit; SourceRef = $ref; RunId = $runId; RunAttempt = $attempt
    }
    function New-MockResult {
        param([string]$Sha = $commit, [string]$Run = $expectedRun,
              [string]$Signer = $expectedSigner, [string]$Hash = $digest,
              [string]$Source = $repository, [string]$Ref = $ref)
        $cert = @{
            issuer = 'https://token.actions.githubusercontent.com'
            githubWorkflowRepository = $Source
            sourceRepositoryURI = "https://github.com/$Source"
            sourceRepositoryDigest = $Sha
            buildSignerURI = $Signer
            buildSignerDigest = $Sha
            sourceRepositoryRef = $Ref
            runInvocationURI = $Run
        }
        $statement = @{
            predicateType = 'https://slsa.dev/provenance/v1'
            subject = @(@{ name = 'composer-windows-x64.zip'; digest = @{ sha256 = $Hash } })
        }
        return ConvertTo-Json -InputObject @(@{
            verificationResult = @{ signature = @{ certificate = $cert }; statement = $statement }
        }) -Depth 10 -Compress
    }
    # Mock only in the policy unit tests; real acceptance uses the GitHub CLI.
    function gh {
        $global:ObservedArgs = @($args)
        $global:LASTEXITCODE = $global:MockExit
        if ($global:MockExit -eq 0) { Write-Output $global:MockResult }
    }
    function Assert-Rejected {
        param([string]$Name)
        try {
            & $verifier @parameters | Out-Null
        } catch {
            Write-Output "PASS rejected $Name"
            return
        }
        throw "FAIL policy accepted invalid $Name"
    }
    $global:MockExit = 0
    $global:MockResult = New-MockResult
    $result = & $verifier @parameters
    if ($result -notmatch '^PASS GitHub attestation identity and SHA256:') {
        throw 'Valid policy fixture was not accepted'
    }
    foreach ($expected in @('--repo','--signer-workflow','--signer-digest',
        '--source-digest','--source-ref','--deny-self-hosted-runners','--format')) {
        if (@($global:ObservedArgs) -cnotcontains $expected) {
            throw "Verifier omitted required CLI option $expected"
        }
    }
    Write-Output 'PASS matching synthetic identity and required CLI options'
    $global:MockResult = New-MockResult -Sha ('0' * 40)
    Assert-Rejected 'signed source commit'
    $global:MockResult = New-MockResult -Run 'https://github.com/other/actions/runs/55/attempts/1'
    Assert-Rejected 'signed workflow run'
    $global:MockResult = New-MockResult -Signer 'https://github.com/attacker/other/.github/workflows/wrong.yml@refs/heads/main'
    Assert-Rejected 'signed workflow path'
    $global:MockResult = New-MockResult -Hash ('0' * 64)
    Assert-Rejected 'signed artifact SHA256'
    $global:MockResult = New-MockResult -Source 'attacker/other'
    Assert-Rejected 'signed repository'
    $global:MockResult = New-MockResult -Ref 'refs/heads/main'
    Assert-Rejected 'signed source ref'
    $global:MockResult = New-MockResult
    $global:MockExit = 1
    Assert-Rejected 'failed signature verification'
    $global:MockExit = 0
    $parameters.Repository = 'attacker/other'
    Assert-Rejected 'unapproved repository input'
    $parameters.Repository = $repository
    $parameters.WorkflowFile = 'wrong.yml'
    Assert-Rejected 'unapproved workflow input'
    Write-Output 'PASS synthetic negative tests; live cryptographic test remains mandatory'
} finally {
    Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
}
