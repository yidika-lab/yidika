# Yidika installer for Windows.
#   irm https://raw.githubusercontent.com/yidika-lab/yidika/main/install.ps1 | iex
# Environment:
#   YIDI_VERSION  release tag to install (default: latest), e.g. v0.1.0
#   YIDI_HOME     install prefix (default: $HOME\.yidi); files go to $YIDI_HOME\bin
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$Repo = 'yidika-lab/yidika'

function Get-Target {
    switch ($env:PROCESSOR_ARCHITECTURE) {
        'AMD64' { return 'x86_64-pc-windows-msvc' }
        default { throw "unsupported CPU architecture: $($env:PROCESSOR_ARCHITECTURE)" }
    }
}

function Get-ReleaseUrl {
    $version = if ($env:YIDI_VERSION) { $env:YIDI_VERSION } else { 'latest' }
    if ($version -eq 'latest') { return "https://github.com/$Repo/releases/latest/download" }
    return "https://github.com/$Repo/releases/download/$version"
}

function Add-ToUserPath([string]$dir) {
    $current = [Environment]::GetEnvironmentVariable('Path', 'User')
    $entries = if ($current) { $current -split ';' } else { @() }
    if ($entries -notcontains $dir) {
        [Environment]::SetEnvironmentVariable('Path', (($entries + $dir) -join ';').Trim(';'), 'User')
        Write-Host "yidi-install: added $dir to your user PATH (open a new terminal)"
    }
}

$target = Get-Target
$asset = "yidi-$target.zip"
$base = Get-ReleaseUrl
$prefix = if ($env:YIDI_HOME) { $env:YIDI_HOME } else { Join-Path $HOME '.yidi' }
$bin = Join-Path $prefix 'bin'
$tmp = Join-Path ([IO.Path]::GetTempPath()) ("yidi-" + [Guid]::NewGuid())
New-Item -ItemType Directory -Path $tmp | Out-Null

try {
    Write-Host "yidi-install: downloading $asset"
    Invoke-WebRequest -UseBasicParsing "$base/$asset" -OutFile (Join-Path $tmp $asset)
    Invoke-WebRequest -UseBasicParsing "$base/$asset.sha256" -OutFile (Join-Path $tmp "$asset.sha256")

    $expected = ((Get-Content (Join-Path $tmp "$asset.sha256") -Raw).Trim() -split '\s+')[0].ToLower()
    $actual = (Get-FileHash -Algorithm SHA256 (Join-Path $tmp $asset)).Hash.ToLower()
    if ($expected -ne $actual) { throw "checksum mismatch for $asset" }

    Expand-Archive -Path (Join-Path $tmp $asset) -DestinationPath $tmp -Force
    New-Item -ItemType Directory -Path $bin -Force | Out-Null
    Copy-Item -Path (Join-Path $tmp "yidi-$target\*") -Destination $bin -Force

    $version = & (Join-Path $bin 'yidi.exe') --version
    Write-Host "yidi-install: installed $version to $bin"
    Add-ToUserPath $bin
}
finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
