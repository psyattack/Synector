#Requires -Version 5.1
<#
.SYNOPSIS
    Synector bootstrap installer.

.DESCRIPTION
    Console entry point behind installer/install.cmd. It resolves the latest GitHub release,
    downloads the requested artifact, verifies its SHA-256 against the digest published by the
    GitHub API and then runs the Setup executable or extracts the portable archive.

    This is a convenience wrapper around the two release artifacts, not a second installer:
    Setup goes through Inno Setup and Portable stays a plain zip.

.PARAMETER Action
    menu (default), setup, portable, uninstall, status or help.

.PARAMETER Silent
    Run Setup or Uninstall unattended (/VERYSILENT /SUPPRESSMSGBOXES /NORESTART).

.PARAMETER Directory
    Setup: installation directory (/DIR). Portable: extraction directory. Defaults to
    %USERPROFILE%\Synector for Portable.

.PARAMETER Version
    Release tag to install (for example v2.9.0 or 2.9.0 instead of the latest release).

.PARAMETER NoLaunch
    Portable only: extract the archive but do not start Synector.exe.

.PARAMETER SkipHashCheck
    Do not verify the SHA-256 digest (only for releases published without a digest).

.EXAMPLE
    installer\install.cmd

.EXAMPLE
    installer\install.cmd setup -Silent -Directory D:\Apps\Synector

.EXAMPLE
    installer\install.cmd portable -Directory D:\Apps\Synector -NoLaunch
#>
[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [ValidateSet('menu', 'setup', 'portable', 'uninstall', 'status', 'help')]
    [string]$Action = 'menu',

    [switch]$Silent,

    [string]$Directory,

    [string]$Version,

    [switch]$NoLaunch,

    [switch]$SkipHashCheck
)

Set-StrictMode -Off
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

# When the script is evaluated from a string (`irm <url> | iex`), PowerShell reports no script path.
# File execution keeps exit codes for automation; remote execution must not close the user's shell.
$script:IsRemoteEntry = -not ($PSCommandPath -and ((Split-Path -Leaf $PSCommandPath) -ieq 'install.ps1'))

$script:Repository = 'psyattack/Synector'
$script:UserAgent = 'Synector-InstallScript'
$script:AppName = 'Synector'

# Hashing and zip extraction use .NET directly: the script must also work when it is started from
# a shell whose PSModulePath points at another PowerShell version's modules.
function Get-Sha256Hex {
    param([string]$Path)

    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        $stream = [System.IO.File]::OpenRead($Path)
        try {
            $hash = $sha.ComputeHash($stream)
        } finally {
            $stream.Dispose()
        }
    } finally {
        $sha.Dispose()
    }

    return ([System.BitConverter]::ToString($hash) -replace '-', '').ToLowerInvariant()
}

function Expand-ZipArchive {
    param([string]$Archive, [string]$Target)

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $fullTarget = [System.IO.Path]::GetFullPath($Target)
    New-Item -ItemType Directory -Force -Path $fullTarget | Out-Null
    # Compare against the target with a trailing separator: a bare prefix test would also accept a
    # sibling directory such as C:\Apps\Synector-evil for a target of C:\Apps\Synector.
    $targetRoot = $fullTarget.TrimEnd([char[]]@('\', '/')) + [System.IO.Path]::DirectorySeparatorChar

    $zip = [System.IO.Compression.ZipFile]::OpenRead($Archive)
    try {
        foreach ($entry in $zip.Entries) {
            $relative = $entry.FullName.Replace('/', [System.IO.Path]::DirectorySeparatorChar)
            $segments = $relative -split '[\\/]'
            # Reject rooted names and parent-directory traversal before resolving the destination.
            if ([System.IO.Path]::IsPathRooted($relative) -or $segments -contains '..') {
                throw "Unsafe archive entry: $($entry.FullName)"
            }
            $destination = [System.IO.Path]::GetFullPath((Join-Path $fullTarget $relative))
            if (-not $destination.StartsWith($targetRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
                throw "Unsafe archive entry: $($entry.FullName)"
            }
            if ([string]::IsNullOrEmpty($entry.Name)) {
                New-Item -ItemType Directory -Force -Path $destination | Out-Null
                continue
            }
            $parent = [System.IO.Path]::GetDirectoryName($destination)
            if ($parent) {
                New-Item -ItemType Directory -Force -Path $parent | Out-Null
            }
            [System.IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $destination, $true)
        }
    } finally {
        $zip.Dispose()
    }
}

function Get-LatestRelease {
    $headers = @{
        'User-Agent' = $script:UserAgent
        'Accept'     = 'application/vnd.github+json'
    }
    if ($Version) {
        $tag = $Version
        if ($tag -notmatch '^v') { $tag = "v$tag" }
        $uri = "https://api.github.com/repos/$script:Repository/releases/tags/$tag"
    } else {
        $uri = "https://api.github.com/repos/$script:Repository/releases/latest"
    }

    try {
        return Invoke-RestMethod -Uri $uri -Headers $headers -UseBasicParsing
    } catch {
        throw "Could not read the release from GitHub ($uri): $($_.Exception.Message)"
    }
}

function Find-ReleaseAsset {
    param($Release, [string]$NamePattern)

    $asset = $Release.assets | Where-Object { $_.name -like $NamePattern } | Select-Object -First 1
    return $asset
}

function Save-ReleaseAsset {
    param($Asset, [string]$TempDir)

    New-Item -ItemType Directory -Force -Path $TempDir | Out-Null
    $path = Join-Path $TempDir $Asset.name
    Write-Host "Downloading $($Asset.name)..."

    $headers = @{ 'User-Agent' = $script:UserAgent }
    Invoke-WebRequest -Uri $Asset.browser_download_url -OutFile $path -Headers $headers -UseBasicParsing

    if (-not $SkipHashCheck) {
        $digest = $null
        if ($Asset.PSObject.Properties['digest']) {
            $digest = [string]$Asset.PSObject.Properties['digest'].Value
        }
        if ($digest -and $digest.StartsWith('sha256:')) {
            $expected = $digest.Substring(7).ToLowerInvariant()
            $actual = Get-Sha256Hex -Path $path
            if ($expected -ne $actual) {
                throw "SHA-256 mismatch for $($Asset.name): expected $expected, got $actual"
            }
            Write-Host 'SHA-256 verified.'
        } else {
            throw "Release asset $($Asset.name) has no published SHA-256 digest. Re-run with -SkipHashCheck to bypass."
        }
    }

    return $path
}

function Resolve-Directory {
    param([string]$Path)

    if (-not $Path) { return $null }
    if (Test-Path -LiteralPath $Path) {
        return (Resolve-Path -LiteralPath $Path).Path
    }
    return [System.IO.Path]::GetFullPath($Path)
}

function Invoke-SetupInstall {
    Write-Host 'Synector Setup'
    $release = Get-LatestRelease
    $asset = Find-ReleaseAsset -Release $release -NamePattern 'Synector-Setup-*-Windows-x64.exe'
    if (-not $asset) {
        throw "Release $($release.tag_name) does not contain a Setup executable yet. Use the portable archive or wait for the next release."
    }

    $tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("Synector-install-" + [guid]::NewGuid().ToString('N'))
    try {
        $setup = Save-ReleaseAsset -Asset $asset -TempDir $tempDir

        $arguments = @()
        if ($Silent) {
            $arguments += '/VERYSILENT'
            $arguments += '/SUPPRESSMSGBOXES'
            $arguments += '/NORESTART'
        }
        $target = Resolve-Directory -Path $Directory
        if ($target) {
            $arguments += "/DIR=`"$target`""
        }

        Write-Host "Running $($asset.name)..."
        $process = Start-Process -FilePath $setup -ArgumentList $arguments -Wait -PassThru
        if ($process.ExitCode -ne 0) {
            throw "Setup exited with code $($process.ExitCode)."
        }
        Write-Host 'Synector is installed.'
    } finally {
        Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}

function Invoke-PortableInstall {
    Write-Host 'Synector Portable'
    $release = Get-LatestRelease
    $asset = Find-ReleaseAsset -Release $release -NamePattern 'Synector-v*-Windows-x64.zip'
    if (-not $asset) {
        throw "Release $($release.tag_name) does not contain a portable archive."
    }

    $target = Resolve-Directory -Path $Directory
    if (-not $target) {
        $target = Join-Path $HOME $script:AppName
    }

    $tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("Synector-install-" + [guid]::NewGuid().ToString('N'))
    try {
        $archive = Save-ReleaseAsset -Asset $asset -TempDir $tempDir
        New-Item -ItemType Directory -Force -Path $target | Out-Null
        Write-Host "Extracting to $target..."
        Expand-ZipArchive -Archive $archive -Target $target

        $exe = Join-Path $target 'Synector.exe'
        if (-not (Test-Path -LiteralPath $exe)) {
            throw "The archive did not contain Synector.exe."
        }
        Write-Host "Synector is ready: $exe"
        if (-not $NoLaunch) {
            Start-Process -FilePath $exe
        }
    } finally {
        Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}

function Find-InstalledCopies {
    $roots = @(
        'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall',
        'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall',
        'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall'
    )

    $found = foreach ($root in $roots) {
        Get-ChildItem -Path $root -ErrorAction SilentlyContinue | ForEach-Object {
            $properties = Get-ItemProperty -Path $_.PSPath -ErrorAction SilentlyContinue
            if ($properties -and $properties.DisplayName -eq $script:AppName) {
                [pscustomobject]@{
                    DisplayVersion   = $properties.DisplayVersion
                    UninstallString  = $properties.UninstallString
                }
            }
        }
    }

    return ($found | Select-Object -First 1)
}

function Split-UninstallCommand {
    param([string]$UninstallString)

    if (-not $UninstallString) { return $null }
    $command = $UninstallString.Trim()
    if ($command.StartsWith('"')) {
        $end = $command.IndexOf('"', 1)
        if ($end -gt 0) {
            return @{ File = $command.Substring(1, $end - 1); Arguments = $command.Substring($end + 1).Trim() }
        }
    }
    $space = $command.IndexOf(' ')
    if ($space -gt 0) {
        return @{ File = $command.Substring(0, $space); Arguments = $command.Substring($space + 1).Trim() }
    }
    return @{ File = $command; Arguments = '' }
}

function Invoke-Uninstall {
    Write-Host 'Synector Uninstall'
    $installed = Find-InstalledCopies
    if (-not $installed) {
        Write-Host 'Synector is not installed through Setup. A portable copy is removed by deleting its folder.'
        return
    }

    $command = Split-UninstallCommand -UninstallString $installed.UninstallString
    if (-not $command) {
        throw 'The uninstall command is missing from the registry entry.'
    }

    $arguments = @()
    if ($command.Arguments) { $arguments += $command.Arguments }
    if ($Silent) {
        $arguments += '/VERYSILENT'
        $arguments += '/SUPPRESSMSGBOXES'
        $arguments += '/NORESTART'
    }

    Write-Host "Running the uninstaller for version $($installed.DisplayVersion)..."
    $process = Start-Process -FilePath $command.File -ArgumentList $arguments -Wait -PassThru
    if ($process.ExitCode -ne 0) {
        throw "The uninstaller exited with code $($process.ExitCode)."
    }
    Write-Host 'Synector has been uninstalled. Settings in %LOCALAPPDATA%\Synector were kept.'
}

function Show-Status {
    $installed = Find-InstalledCopies
    if ($installed) {
        Write-Host "Setup install: version $($installed.DisplayVersion)"
    } else {
        Write-Host 'Setup install: not installed'
    }

    try {
        $release = Get-LatestRelease
        Write-Host "Latest release: $($release.tag_name)"
    } catch {
        Write-Host "Latest release: unavailable ($($_.Exception.Message))"
    }

    if ($env:LOCALAPPDATA) {
        $dataDir = Join-Path $env:LOCALAPPDATA $script:AppName
        if (Test-Path -LiteralPath $dataDir) {
            Write-Host "User data: $dataDir"
        }
    }
}

function Show-Help {
    Write-Host @'
Synector bootstrap installer

Usage:
  installer\install.cmd [action] [options]

Actions:
  menu       Interactive menu (default)
  setup      Download and run the Setup executable
  portable   Download the portable archive and extract it
  uninstall  Run the uninstaller of an installed copy
  status     Show installed and latest versions
  help       Show this help

Options:
  -Silent              Unattended Setup/Uninstall
  -Directory <path>    Setup destination or Portable target folder
  -Version <tag>       Install a specific release instead of the latest
  -NoLaunch            Portable: do not start Synector after extracting
  -SkipHashCheck       Skip SHA-256 verification

Examples:
  installer\install.cmd
  installer\install.cmd setup -Silent
  installer\install.cmd portable -Directory D:\Apps\Synector -NoLaunch
  installer\install.cmd uninstall -Silent

Remote execution (no repository checkout):
  irm https://raw.githubusercontent.com/psyattack/Synector/main/installer/install.ps1 | iex

Remote execution with an action:
  & ([scriptblock]::Create((irm "https://raw.githubusercontent.com/psyattack/Synector/main/installer/install.ps1"))) setup -Silent
'@
}

function Start-Menu {
    if ([Console]::IsInputRedirected) {
        throw 'No console input is available. Use: & ([scriptblock]::Create((irm <install.ps1 url>))) setup -Silent'
    }

    while ($true) {
        Write-Host ''
        Write-Host 'Synector installer' -ForegroundColor Cyan
        Write-Host '  1. Install (Setup)'
        Write-Host '  2. Portable'
        Write-Host '  3. Uninstall'
        Write-Host '  4. Status'
        Write-Host '  0. Exit'

        $choice = Read-Host 'Select an option'
        if (-not $choice) { return }
        switch ($choice.Trim()) {
            '1' {
                Invoke-SetupInstall
            }
            '2' {
                $default = Join-Path $HOME $script:AppName
                $answer = Read-Host "Target folder [$default]"
                if ($answer.Trim()) { $Directory = $answer.Trim() } else { $Directory = $default }
                Invoke-PortableInstall
            }
            '3' {
                Invoke-Uninstall
            }
            '4' {
                Show-Status
            }
            '0' {
                return
            }
            default {
                Write-Host 'Unknown option.'
            }
        }
    }
}

try {
    switch ($Action) {
        'setup'     { Invoke-SetupInstall }
        'portable'  { Invoke-PortableInstall }
        'uninstall' { Invoke-Uninstall }
        'status'    { Show-Status }
        'help'      { Show-Help }
        default     { Start-Menu }
    }
} catch {
    Write-Host "Error: $($_.Exception.Message)" -ForegroundColor Red
    if ($script:IsRemoteEntry) {
        Write-Host 'The script stopped; your PowerShell session is still running.'
    } else {
        exit 1
    }
}
