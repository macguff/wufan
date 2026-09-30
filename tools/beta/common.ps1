# Shared by the ordinary-user controller and the same-user UAC registration helper.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$script:BetaClsid = '{58F68769-239A-4DCA-854E-57AA887E979B}'
$script:BetaComKey = "Software\Classes\CLSID\$script:BetaClsid\InprocServer32"
$script:BetaOwnerKey = 'SOFTWARE\Wufan\TechnicalBeta'
$script:BetaRunName = 'WufanTechnicalBeta'

function Get-UserSid { [Security.Principal.WindowsIdentity]::GetCurrent().User.Value }
function Test-Administrator {
    $p = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
    $p.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}
function Get-SafePath([string]$Path) {
    if ([string]::IsNullOrWhiteSpace($Path) -or $Path -match '["\x00-\x1f]' -or $Path.StartsWith('\\')) {
        throw "An absolute local path without quotes/control characters is required: $Path"
    }
    if (-not [IO.Path]::IsPathRooted($Path)) { throw "Relative path rejected: $Path" }
    $full = [IO.Path]::GetFullPath($Path).TrimEnd('\')
    $cursor = $full
    while ($cursor) {
        if (Test-Path -LiteralPath $cursor) {
            $item = Get-Item -LiteralPath $cursor -Force
            if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Reparse point rejected: $cursor" }
        }
        $parent = [IO.Path]::GetDirectoryName($cursor)
        if ($parent -eq $cursor) { break }
        $cursor = $parent
    }
    $full
}
function Get-ChildPath([string]$Root, [string]$Relative) {
    if ([IO.Path]::IsPathRooted($Relative) -or $Relative.Contains(':') -or $Relative -match '(^|[\\/])\.\.([\\/]|$)') { throw "Unsafe relative path: $Relative" }
    $rootPath = Get-SafePath $Root
    $full = Get-SafePath (Join-Path $rootPath $Relative)
    if (-not $full.StartsWith($rootPath + '\', [StringComparison]::OrdinalIgnoreCase)) { throw "Path escapes root: $Relative" }
    $full
}
function Write-AtomicText([string]$Path, [string]$Text) {
    $path = Get-SafePath $Path
    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($path)) | Out-Null
    $temp = $path + '.' + [Guid]::NewGuid().ToString('N') + '.tmp'
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes($Text)
    $stream = [IO.FileStream]::new($temp, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try { $stream.Write($bytes, 0, $bytes.Length); $stream.Flush($true) } finally { $stream.Dispose() }
    if (Test-Path -LiteralPath $path) { [IO.File]::Replace($temp, $path, $null) }
    else { [IO.File]::Move($temp, $path) }
}
function Write-Json([string]$Path, $Value) { Write-AtomicText $Path ($Value | ConvertTo-Json -Depth 16) }
function Read-Json([string]$Path) {
    if (Test-Path -LiteralPath $Path) { Get-Content -LiteralPath $Path -Raw -Encoding UTF8 | ConvertFrom-Json }
}
function Get-BetaFileHash([string]$LiteralPath) {
    $stream = [IO.File]::OpenRead((Get-SafePath $LiteralPath))
    $algorithm = [Security.Cryptography.SHA256]::Create()
    try { [pscustomobject]@{ Hash = ([BitConverter]::ToString($algorithm.ComputeHash($stream))).Replace('-','').ToLowerInvariant() } }
    finally { $algorithm.Dispose(); $stream.Dispose() }
}
function Quote-Argument([string]$Value) {
    if ($Value -match '["\x00-\x1f]' -or $Value.EndsWith('\')) { throw "Unsupported command argument: $Value" }
    '"' + $Value + '"'
}
function Get-PowerShell { Join-Path $env:WINDIR 'System32\WindowsPowerShell\v1.0\powershell.exe' }
function Get-ComDll {
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($script:BetaComKey)
    if ($null -eq $key) { return $null }
    try { $key.GetValue('') } finally { $key.Dispose() }
}
function Get-StartupValue {
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software\Microsoft\Windows\CurrentVersion\Run')
    if ($null -eq $key) { return $null }
    try { $key.GetValue($script:BetaRunName) } finally { $key.Dispose() }
}
function Set-StartupValue($Value) {
    $key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Software\Microsoft\Windows\CurrentVersion\Run')
    try {
        if ($null -eq $Value) { $key.DeleteValue($script:BetaRunName, $false) }
        else { $key.SetValue($script:BetaRunName, [string]$Value, [Microsoft.Win32.RegistryValueKind]::String) }
    } finally { $key.Dispose() }
}
function Assert-Package([string]$Root) {
    $rootPath = Get-SafePath $Root
    $manifestPath = Get-ChildPath $rootPath 'manifest.json'
    $manifest = Read-Json $manifestPath
    if ($null -eq $manifest -or $manifest.format -ne 1 -or $manifest.architecture -ne 'windows-x64' -or
        $manifest.brokerFeatures -ne 'default' -or $manifest.product -ne 'WufanTechnicalBeta') {
        throw 'Unsupported package manifest; fault builds are never accepted.'
    }
    $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($file in $manifest.files) {
        if ($file.path.Contains('\') -or @($file.path.Split('/') | Where-Object { $_ -in @('','.', '..') }).Count) {
            throw "Noncanonical manifest path: $($file.path)"
        }
        if (-not $seen.Add([string]$file.path) -or $file.path -eq 'manifest.json') { throw 'Duplicate manifest path' }
        $path = Get-ChildPath $rootPath $file.path
        if ((Get-Item -LiteralPath $path).Length -ne $file.size -or
            (Get-BetaFileHash -LiteralPath $path).Hash -ne $file.sha256) { throw "Package hash mismatch: $($file.path)" }
    }
    foreach ($required in @('ime_windows_tsf.dll','ime-broker.exe','runtime/native/dist/lib/rime.dll',
        'runtime/shared/wufan_pinyin.schema.yaml','tools/wufan.ps1','tools/common.ps1','tools/register.ps1')) {
        if (-not $seen.Contains($required)) { throw "Required payload missing: $required" }
    }
    $directories = [Collections.Generic.Stack[string]]::new()
    $directories.Push($rootPath)
    while ($directories.Count) {
        foreach ($item in Get-ChildItem -LiteralPath $directories.Pop() -Force) {
            if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Reparse payload rejected: $($item.FullName)" }
            if ($item.PSIsContainer) { $directories.Push($item.FullName); continue }
            $relative = $item.FullName.Substring($rootPath.Length + 1).Replace('\','/')
            if ($relative -ne 'manifest.json' -and -not $seen.Contains($relative)) { throw "Unlisted package file: $relative" }
        }
    }
    $manifest
}
