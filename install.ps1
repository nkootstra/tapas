[CmdletBinding()]
param(
    [switch]$CleanDevBuilds,
    [switch]$DryRun,
    [string]$InstallDir,
    [string]$Version,
    [string]$Repository = 'nkootstra/tapas'
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

if ($CleanDevBuilds) {
    if ($Version) {
        Write-Error '-Version cannot be combined with -CleanDevBuilds'
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
    exit 0
}

if ($DryRun) {
    Write-Error '-DryRun requires -CleanDevBuilds'
    exit 2
}

$api = "https://api.github.com/repos/$Repository"
$headers = @{ 'Accept' = 'application/vnd.github+json' }
try {
    if ($Version) {
        $releaseTag = if ($Version.StartsWith('v')) { $Version } else { "v$Version" }
        $release = Invoke-RestMethod "$api/releases/tags/$releaseTag" -Headers $headers
    } else {
        $release = @(
            Invoke-RestMethod "$api/releases?per_page=100" -Headers $headers |
                Where-Object { -not $_.draft -and -not $_.prerelease } |
                Sort-Object { [DateTime]$_.published_at } -Descending
        )[0]
        if (-not $release) {
            throw 'no stable release found'
        }
        $releaseTag = $release.tag_name
    }
} catch {
    Write-Error "could not find release: $($_.Exception.Message)"
    exit 1
}

$target = 'x86_64-pc-windows-msvc'
$asset = "tapas-$target.zip"
$checksumAsset = 'SHA256SUMS'
$temporary = Join-Path ([IO.Path]::GetTempPath()) ("tapas-install-" + [Guid]::NewGuid())
New-Item -ItemType Directory -Path $temporary | Out-Null
try {
    $archivePath = Join-Path $temporary $asset
    $checksumsPath = Join-Path $temporary $checksumAsset
    Invoke-WebRequest "https://github.com/$Repository/releases/download/$releaseTag/$asset" -OutFile $archivePath
    Invoke-WebRequest "https://github.com/$Repository/releases/download/$releaseTag/$checksumAsset" -OutFile $checksumsPath

    $expected = (Select-String -Path $checksumsPath -Pattern "^([0-9a-fA-F]{64})\s+$([regex]::Escape($asset))$").Matches.Groups[1].Value
    if (-not $expected) {
        throw "release checksum missing for $asset"
    }
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $archivePath).Hash
    if ($expected.ToLowerInvariant() -ne $actual.ToLowerInvariant()) {
        throw "release checksum mismatch for $asset"
    }

    $unpacked = Join-Path $temporary 'unpacked'
    Expand-Archive -LiteralPath $archivePath -DestinationPath $unpacked -Force
    $metadataPath = Join-Path $unpacked 'BUILD-METADATA.json'
    $metadata = Get-Content -Raw -LiteralPath $metadataPath | ConvertFrom-Json
    if ($metadata.target -ne $target) {
        throw 'release target metadata mismatch'
    }
    if ($metadata.version -ne $releaseTag.TrimStart('v')) {
        throw 'release version metadata mismatch'
    }
    $binaryPath = Join-Path $unpacked 'tapas.exe'
    if (-not (Test-Path -LiteralPath $binaryPath -PathType Leaf)) {
        throw 'release binary missing'
    }

    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    $destination = Join-Path $InstallDir 'tapas.exe'
    Copy-Item -LiteralPath $binaryPath -Destination $destination -Force
    Write-Output "installed $releaseTag as $destination"
    Write-Output "version: $($metadata.version_label)"
    Write-Output 'run: tapas.exe --version'
} catch {
    Write-Error $_.Exception.Message
    exit 1
} finally {
    Remove-Item -LiteralPath $temporary -Recurse -Force -ErrorAction SilentlyContinue
}
