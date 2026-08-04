[CmdletBinding()]
param(
    [switch]$CleanDevBuilds,
    [switch]$DryRun,
    [string]$InstallDir
)

if ([string]::IsNullOrWhiteSpace($InstallDir)) {
    $InstallDir = if (-not [string]::IsNullOrWhiteSpace($env:TAPAS_INSTALL_DIR)) {
        $env:TAPAS_INSTALL_DIR
    } elseif (-not [string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
        Join-Path $env:LOCALAPPDATA 'Programs\tapas'
    } else {
        Join-Path $HOME '.tapas'
    }
}

if (-not $CleanDevBuilds) {
    Write-Error 'usage: install.ps1 -CleanDevBuilds [-DryRun] [-InstallDir PATH]'
    exit 2
}

if (-not (Test-Path -LiteralPath $InstallDir -PathType Container)) {
    Write-Output "no local development builds found in $InstallDir"
    exit 0
}

$builds = @(Get-ChildItem -LiteralPath $InstallDir -File -Filter 'tapas-pr-*')
if ($builds.Count -eq 0) {
    Write-Output "no local development builds found in $InstallDir"
    exit 0
}

foreach ($build in $builds) {
    if ($DryRun) {
        Write-Output "would remove $($build.FullName)"
    } else {
        Remove-Item -LiteralPath $build.FullName -Force
        Write-Output "removed $($build.FullName)"
    }
}
