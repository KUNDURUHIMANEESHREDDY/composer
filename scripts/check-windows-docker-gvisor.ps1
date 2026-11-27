# Non-destructive host acceptance check, not an installer.
# Requires a connected Windows machine with Docker Desktop already configured
# for Linux containers and the third-party gVisor runsc runtime.
# Does not pull images, install software, change daemon settings, or elevate.
param(
    [string]$EvidencePath = "composer-windows-docker-gvisor-evidence.json"
)
$ErrorActionPreference = "Stop"
# PowerShell 5 treats native stderr as a terminating error under Stop before
# LASTEXITCODE can classify a missing Docker engine. Do not expose native stderr
# or silently turn a failed probe into a successful one.
function Invoke-DockerProbe {
    param([string[]]$DockerArgs)
    $prior = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $output = & docker @DockerArgs 2>$null
        $code = $LASTEXITCODE
        return [pscustomobject]@{
            ExitCode = $code
            Output = ($output | Out-String).Trim()
        }
    }
    finally {
        $ErrorActionPreference = $prior
    }
}
$evidence = [ordered]@{
    schema_version = 1
    host = "Windows"
    docker_cli_found = $false
    docker_context = $null
    daemon_os_linux = $false
    docker_desktop_daemon = $false
    runsc_registered = $false
    image_cached = $false
    runsc_smoke_passed = $false
    verified_windows_docker_desktop_gvisor = $false
    disposition = "not_verified"
}
try {
    if (-not (Get-Command docker -ErrorAction SilentlyContinue)) {
        $evidence.disposition = "docker_cli_not_found"
    }
    else {
        $evidence.docker_cli_found = $true
        $context = Invoke-DockerProbe -DockerArgs @("context", "show")
        if ($context.ExitCode -ne 0) { throw "docker_context_unavailable" }
        $evidence.docker_context = $context.Output

        $daemon = Invoke-DockerProbe -DockerArgs @("info", "--format", "{{.OSType}}")
        if ($daemon.ExitCode -ne 0) { throw "docker_engine_unavailable" }
        $evidence.daemon_os_linux = $daemon.Output -eq "linux"
        if (-not $evidence.daemon_os_linux) { throw "docker_is_not_using_linux_containers" }

        # A generic remote Linux Docker daemon is not proof of Docker Desktop.
        $daemonProduct = Invoke-DockerProbe -DockerArgs @("info", "--format", "{{.OperatingSystem}}")
        if ($daemonProduct.ExitCode -ne 0) { throw "docker_engine_identity_unavailable" }
        $evidence.docker_desktop_daemon = $daemonProduct.Output -match "Docker Desktop"
        if (-not $evidence.docker_desktop_daemon) { throw "docker_engine_is_not_docker_desktop" }

        $registered = Invoke-DockerProbe -DockerArgs @("info", "--format", "{{json .Runtimes}}")
        if ($registered.ExitCode -ne 0) { throw "runtime_registry_unavailable" }
        $runtimes = $registered.Output | ConvertFrom-Json
        $evidence.runsc_registered = @($runtimes.PSObject.Properties.Name) -contains "runsc"
        if (-not $evidence.runsc_registered) { throw "runsc_runtime_not_registered" }

        $cached = Invoke-DockerProbe -DockerArgs @("image", "inspect", "rust:1.99-slim", "--format", "{{.Id}}")
        $evidence.image_cached = $cached.ExitCode -eq 0
        if (-not $evidence.image_cached) { throw "rust_verifier_image_not_cached_no_auto_download" }

        $smoke = Invoke-DockerProbe -DockerArgs @("run", "--rm", "--pull=never", "--runtime=runsc", "--network=none", "--read-only", "--cap-drop=ALL", "--security-opt=no-new-privileges", "rust:1.99-slim", "sh", "-c", "printf COMPOSER_WINDOWS_GVISOR_RUNSC_OK")
        $evidence.runsc_smoke_passed = (
            $smoke.ExitCode -eq 0 -and
            $smoke.Output.Contains("COMPOSER_WINDOWS_GVISOR_RUNSC_OK")
        )
        if (-not $evidence.runsc_smoke_passed) { throw "runsc_smoke_failed" }
        $evidence.verified_windows_docker_desktop_gvisor = $true
        $evidence.disposition = "host_runsc_smoke_passed"
    }
}
catch {
    # Errors are classified, but raw Docker stderr (which might include local
    # usernames, registry URLs, etc.) is never recorded as evidence.
    $reason = [string]$_.Exception.Message
    $recognized = @(
        "docker_context_unavailable",
        "docker_engine_unavailable",
        "docker_is_not_using_linux_containers",
        "docker_engine_identity_unavailable",
        "docker_engine_is_not_docker_desktop",
        "runtime_registry_unavailable",
        "runsc_runtime_not_registered",
        "rust_verifier_image_not_cached_no_auto_download",
        "runsc_smoke_failed"
    )
    if ($recognized -contains $reason) {
        $evidence.disposition = $reason
    }
    else {
        $evidence.disposition = "unclassified_host_error"
    }
}
finally {
    $evidence | ConvertTo-Json -Depth 4 | Set-Content -Encoding UTF8 -Path $EvidencePath
    Write-Output ($evidence | ConvertTo-Json -Compress -Depth 4)
}
if (-not $evidence.verified_windows_docker_desktop_gvisor) { exit 2 }
exit 0
