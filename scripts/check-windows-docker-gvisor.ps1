# Non-destructive host acceptance check, not an installer.
# Requires a connected Windows machine with Docker Desktop already configured
# for Linux containers and the third-party gVisor runsc runtime.
# Does not pull images, install software, change daemon settings, or elevate.
param(
    [string]$EvidencePath = "composer-windows-docker-gvisor-evidence.json"
)
$ErrorActionPreference = "Stop"
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
        $context = & docker context show 2>&1
        if ($LASTEXITCODE -ne 0) { throw "docker_context_unavailable" }
        $evidence.docker_context = ($context | Out-String).Trim()

        $daemon = & docker info --format '{{.OSType}}' 2>&1
        if ($LASTEXITCODE -ne 0) { throw "docker_engine_unavailable" }
        $evidence.daemon_os_linux = ($daemon | Out-String).Trim() -eq "linux"
        if (-not $evidence.daemon_os_linux) { throw "docker_is_not_using_linux_containers" }

        # A generic remote Linux Docker daemon is not proof of Docker Desktop.
        $daemonProduct = & docker info --format '{{.OperatingSystem}}' 2>&1
        if ($LASTEXITCODE -ne 0) { throw "docker_engine_identity_unavailable" }
        $evidence.docker_desktop_daemon = ($daemonProduct | Out-String).Trim() -match "Docker Desktop"
        if (-not $evidence.docker_desktop_daemon) { throw "docker_engine_is_not_docker_desktop" }

        $registered = & docker info --format '{{json .Runtimes}}' 2>&1
        if ($LASTEXITCODE -ne 0) { throw "runtime_registry_unavailable" }
        $runtimes = ($registered | Out-String).Trim() | ConvertFrom-Json
        $evidence.runsc_registered = @($runtimes.PSObject.Properties.Name) -contains "runsc"
        if (-not $evidence.runsc_registered) { throw "runsc_runtime_not_registered" }

        $cached = & docker image inspect rust:1.99-slim --format '{{.Id}}' 2>&1
        $evidence.image_cached = $LASTEXITCODE -eq 0
        if (-not $evidence.image_cached) { throw "rust_verifier_image_not_cached_no_auto_download" }

        $output = & docker run --rm --pull=never --runtime=runsc --network=none --read-only --cap-drop=ALL --security-opt=no-new-privileges rust:1.99-slim sh -c 'printf COMPOSER_WINDOWS_GVISOR_RUNSC_OK' 2>&1
        $evidence.runsc_smoke_passed = (
            $LASTEXITCODE -eq 0 -and
            ($output | Out-String).Contains("COMPOSER_WINDOWS_GVISOR_RUNSC_OK")
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
