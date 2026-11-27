# Non-destructive host acceptance check, not an installer.
# Requires a connected Windows machine with Docker Desktop already configured
# for Linux containers and the third-party gVisor runsc runtime.
# Does not pull images, install software, change daemon settings, or elevate.
param(
    [string]$EvidencePath = "composer-windows-docker-gvisor-evidence.json"
)
$ErrorActionPreference = "Stop"

# Windows PowerShell 5.1 converts native stderr to a terminating
# NativeCommandError under Stop. Use the existing Docker CLI in a narrow
# helper that captures ONLY stdout and exit status; never record raw stderr
# containing local usernames, registry names or daemon paths.
function Invoke-DockerQuiet {
    param([string[]]$DockerArguments)
    $previous = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $stdout = & docker @DockerArguments 2>$null
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previous
    }
    return [PSCustomObject]@{
        Ok = ($exitCode -eq 0)
        Text = ($stdout | Out-String).Trim()
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
        $context = Invoke-DockerQuiet -DockerArguments @("context", "show")
        if (-not $context.Ok) { throw "docker_context_unavailable" }
        $evidence.docker_context = $context.Text

        $daemon = Invoke-DockerQuiet -DockerArguments @("info", "--format", "{{.OSType}}")
        if (-not $daemon.Ok) { throw "docker_engine_unavailable" }
        $evidence.daemon_os_linux = $daemon.Text -eq "linux"
        if (-not $evidence.daemon_os_linux) { throw "docker_is_not_using_linux_containers" }

        # A generic remote Linux Docker daemon is not proof of Docker Desktop.
        $daemonProduct = Invoke-DockerQuiet -DockerArguments @("info", "--format", "{{.OperatingSystem}}")
        if (-not $daemonProduct.Ok) { throw "docker_engine_identity_unavailable" }
        $evidence.docker_desktop_daemon = $daemonProduct.Text -match "Docker Desktop"
        if (-not $evidence.docker_desktop_daemon) { throw "docker_engine_is_not_docker_desktop" }

        $registered = Invoke-DockerQuiet -DockerArguments @("info", "--format", "{{json .Runtimes}}")
        if (-not $registered.Ok) { throw "runtime_registry_unavailable" }
        $runtimes = $registered.Text | ConvertFrom-Json
        $evidence.runsc_registered = @($runtimes.PSObject.Properties.Name) -contains "runsc"
        if (-not $evidence.runsc_registered) { throw "runsc_runtime_not_registered" }

        $cached = Invoke-DockerQuiet -DockerArguments @("image", "inspect", "rust:1.99-slim", "--format", "{{.Id}}")
        $evidence.image_cached = $cached.Ok
        if (-not $evidence.image_cached) { throw "rust_verifier_image_not_cached_no_auto_download" }

        $output = Invoke-DockerQuiet -DockerArguments @(
            "run", "--rm", "--pull=never", "--runtime=runsc", "--network=none",
            "--read-only", "--cap-drop=ALL", "--security-opt=no-new-privileges",
            "rust:1.99-slim", "sh", "-c", "printf COMPOSER_WINDOWS_GVISOR_RUNSC_OK"
        )
        $evidence.runsc_smoke_passed = (
            $output.Ok -and $output.Text.Contains("COMPOSER_WINDOWS_GVISOR_RUNSC_OK")
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
