[CmdletBinding()]
param(
    [ValidatePattern('^[0-9A-Za-z][0-9A-Za-z._-]{0,60}$')][string]$Version = '0.1.0-beta.1',
    [string]$OutputDirectory,
    [switch]$ReleaseReady
)
$ErrorActionPreference = 'Stop'
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
. (Join-Path $repoRoot 'tools\beta\common.ps1')
if (-not [Environment]::Is64BitProcess) { throw 'Use 64-bit PowerShell in an x64 Visual Studio Developer shell.' }
if (-not $env:VSCMD_ARG_TGT_ARCH -or $env:VSCMD_ARG_TGT_ARCH -ne 'x64') {
    throw 'Run from an x64 Visual Studio Developer shell (VsDevCmd.bat -arch=x64 -host_arch=x64).'
}
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $repoRoot 'target\beta-packages' }
$outputRoot = Get-SafePath $OutputDirectory
$runtime = Join-Path $repoRoot 'target\rime-runtime'
$buildRoot = Join-Path $repoRoot 'target\beta-build'
Push-Location $repoRoot
try {
    # A separate target directory and explicit no-default-features invocation prevent
    # accidentally selecting the acceptance-only Broker from target/fault-acceptance.
    & cargo build --locked --release --target x86_64-pc-windows-msvc --target-dir $buildRoot --no-default-features -p ime-broker -p ime-windows-tsf
    if ($LASTEXITCODE -ne 0) { throw 'Beta build failed.' }
    $metadataText = & cargo metadata --locked --offline --filter-platform x86_64-pc-windows-msvc --format-version 1
    if ($LASTEXITCODE -ne 0) { throw 'Locked dependency metadata unavailable.' }
    $metadata = ($metadataText -join "`n") | ConvertFrom-Json
    $commit = (& git rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Git revision unavailable.' }
    $sourcePaths = @(& git -c core.quotepath=false ls-files --cached --others --exclude-standard)
    if ($LASTEXITCODE -ne 0) { throw 'Source inventory unavailable.' }
} finally { Pop-Location }

[IO.Directory]::CreateDirectory($outputRoot) | Out-Null
$staging = Get-ChildPath $outputRoot ('.staging-' + [Guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($staging) | Out-Null
function Copy-Payload([string]$Source, [string]$Relative) {
    $destination = Get-ChildPath $staging $Relative
    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($destination)) | Out-Null
    Copy-Item -LiteralPath $Source -Destination $destination
}
function Require-Digest([string]$Path, [string]$Expected) {
    if ((Get-BetaFileHash -LiteralPath $Path).Hash -ne $Expected) { throw "Pinned runtime mismatch: $Path" }
}
Require-Digest (Join-Path $runtime 'native\dist\lib\rime.dll') '86b4c7357d4c6d293ce5589b234d8859ca2ac30923a03bedfa3926eeaf97fb0b'
Require-Digest (Join-Path $runtime 'shared\luna_pinyin.dict.yaml') '75bcf6eb3ff62b129882ed89cc22b2d80a5347aa72bcfa2ccc839bac298e7314'
Require-Digest (Join-Path $runtime 'shared\essay.txt') '3e8512ebaa6657e35961b1f5762060717c647e3bf471d5e3b86e783b4e934ffb'
Require-Digest (Join-Path $runtime 'deps.7z') '9ef5608d8a54ff52bbad7a9b4128de42b232f8e3dd1f5fd3bff42a0b1bacd7e8'
Copy-Payload (Join-Path $buildRoot 'x86_64-pc-windows-msvc\release\ime-broker.exe') 'ime-broker.exe'
Copy-Payload (Join-Path $buildRoot 'x86_64-pc-windows-msvc\release\ime_windows_tsf.dll') 'ime_windows_tsf.dll'
Copy-Payload (Join-Path $runtime 'native\dist\lib\rime.dll') 'runtime/native/dist/lib/rime.dll'
foreach ($name in @('essay.txt','luna_pinyin.dict.yaml')) { Copy-Payload (Join-Path $runtime "shared\$name") "runtime/shared/$name" }
foreach ($name in @('default.yaml','wufan_pinyin.schema.yaml')) { Copy-Payload (Join-Path $repoRoot "config\rime\$name") "runtime/shared/$name" }
# Extract only OpenCC data from the hashed archive into a NEW directory. Do not copy
# potentially stale files or user/deployed dictionaries from the development runtime.
if (-not (Get-Command 7z -ErrorAction SilentlyContinue)) { throw '7z.exe is needed by the package builder.' }
$openccStage = Get-ChildPath $staging 'runtime'
& 7z x (Join-Path $runtime 'deps.7z') "-o$openccStage" 'share/opencc/*' -y | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'Pinned OpenCC data extraction failed.' }
Move-Item -LiteralPath (Get-ChildPath $staging 'runtime\share\opencc') -Destination (Get-ChildPath $staging 'runtime\shared\opencc')
# The emptied extraction directory is known, resolved, nonrecursive and inside staging.
[IO.Directory]::Delete((Get-ChildPath $staging 'runtime\share'))
foreach ($name in @('common.ps1','register.ps1','wufan.ps1')) { Copy-Payload (Join-Path $repoRoot "tools\beta\$name") "tools/$name" }
Copy-Payload (Join-Path $repoRoot 'BETA_PACKAGE.md') 'README.md'
Copy-Payload (Join-Path $repoRoot 'crates\rime-sys\vendor\LICENSE') 'licenses/librime-LICENSE'
Copy-Payload (Join-Path $repoRoot 'config\beta\native-licenses.json') 'licenses/native-inventory.json'
Copy-Payload (Join-Path $repoRoot 'config\beta\release-gates.json') 'build/release-gates.json'
foreach ($file in Get-ChildItem -LiteralPath (Join-Path $repoRoot 'config\beta\licenses') -File) {
    Copy-Payload $file.FullName "licenses/native/$($file.Name)"
}
Copy-Payload (Join-Path $runtime 'native\version-info.txt') 'build/librime-version-info.txt'
Copy-Payload (Join-Path $repoRoot 'Cargo.lock') 'build/Cargo.lock'
Copy-Payload (Join-Path $repoRoot 'rust-toolchain.toml') 'build/rust-toolchain.toml'

# Include exact locked Rust dependency license/notice texts. Inventory all resolved
# packages conservatively, including build dependencies; this is not a legal audit.
$rustInventory = @()
$licenseMissing = @()
foreach ($package in @($metadata.packages | Where-Object { $null -ne $_.source } | Sort-Object name,version)) {
    $directory = Split-Path $package.manifest_path -Parent
    $texts = @(Get-ChildItem -LiteralPath $directory -File | Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE)' })
    if ($package.license_file) {
        $explicit = Get-Item -LiteralPath (Join-Path $directory $package.license_file)
        $texts = @($texts + $explicit | Sort-Object FullName -Unique)
    }
    if ($texts.Count -eq 0) { $licenseMissing += "$($package.name)@$($package.version)" }
    foreach ($file in $texts) { Copy-Payload $file.FullName "licenses/rust/$($package.name)-$($package.version)/$($file.Name)" }
    $rustInventory += [ordered]@{ name = $package.name; version = $package.version; license = $package.license;
        source = $package.source; texts = @($texts | ForEach-Object { $_.Name }) }
}
Write-Json (Get-ChildPath $staging 'licenses\rust-inventory.json') $rustInventory
$nativeInventory = Read-Json (Join-Path $repoRoot 'config\beta\native-licenses.json')
$licenseComplete = $nativeInventory.reviewComplete -and $licenseMissing.Count -eq 0
$releaseGates = Read-Json (Join-Path $repoRoot 'config\beta\release-gates.json')
if ($releaseGates.format -ne 1) { throw 'Unsupported acceptance gate format.' }
$pendingGates = @('chromium','installationTransactions','cleanMachine','workerLifecycle','repeatedArchiveIdentity' |
    Where-Object { $releaseGates.$_ -ne 'accepted' })
$redistributable = $licenseComplete -and $pendingGates.Count -eq 0
if ($ReleaseReady -and -not $redistributable) { throw "Release gate closed: license review=$licenseComplete; pending=$($pendingGates -join ','). Staging is retained for inspection." }

[Array]::Sort($sourcePaths, [StringComparer]::Ordinal)
$sourceRecords = foreach ($relative in $sourcePaths) {
    $path = Get-ChildPath $repoRoot $relative
    if (Test-Path -LiteralPath $path -PathType Leaf) { $relative.Replace('\','/') + ' ' + (Get-BetaFileHash -LiteralPath $path).Hash.ToLowerInvariant() }
}
$sha = [Security.Cryptography.SHA256]::Create()
try { $sourceDigest = ([BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes(($sourceRecords -join "`n"))))).Replace('-','').ToLowerInvariant() }
finally { $sha.Dispose() }
$paths = [string[]]@(Get-ChildItem -LiteralPath $staging -Recurse -File | ForEach-Object { $_.FullName.Substring($staging.Length + 1).Replace('\','/') })
[Array]::Sort($paths, [StringComparer]::Ordinal)
$records = foreach ($relative in $paths) {
    $path = Get-ChildPath $staging $relative
    [ordered]@{ path = $relative; size = (Get-Item -LiteralPath $path).Length; sha256 = (Get-BetaFileHash -LiteralPath $path).Hash.ToLowerInvariant() }
}
$manifest = [ordered]@{ format = 1; product = 'WufanTechnicalBeta'; version = $Version;
    architecture = 'windows-x64'; brokerFeatures = 'default'; librime = '1.17.0'; schema = 'wufan_pinyin';
    learning = $false; sourceCommit = $commit; sourceTreeSha256 = $sourceDigest;
    distributionReady = [bool]$redistributable; licenseReviewComplete = [bool]$licenseComplete;
    acceptance = $releaseGates; missingRustLicenseTexts = @($licenseMissing); files = @($records) }
Write-Json (Get-ChildPath $staging 'manifest.json') $manifest
$null = Assert-Package $staging
$manifestDigest = (Get-BetaFileHash -LiteralPath (Join-Path $staging 'manifest.json')).Hash.ToLowerInvariant()
$packageName = "wufan-$Version-windows-x64-$($manifestDigest.Substring(0,12))"
$packageRoot = Get-ChildPath $outputRoot $packageName
if (Test-Path -LiteralPath $packageRoot) {
    $null = Assert-Package $packageRoot
    if ((Get-BetaFileHash -LiteralPath (Join-Path $packageRoot 'manifest.json')).Hash.ToLowerInvariant() -ne $manifestDigest) { throw 'Existing package name collision.' }
    # No recursive deletion. Repeated invocation may retain a staging directory;
    # the canonical payload and ZIP are still identical.
} else { [IO.Directory]::Move($staging, $packageRoot) }

Add-Type -AssemblyName System.IO.Compression
$zipPath = Get-ChildPath $outputRoot ($packageName + '.zip')
$zipTemp = $zipPath + '.' + [Guid]::NewGuid().ToString('N') + '.tmp'
$stream = [IO.File]::Open($zipTemp, 'CreateNew', 'ReadWrite', 'None')
$zip = [IO.Compression.ZipArchive]::new($stream, [IO.Compression.ZipArchiveMode]::Create, $false, [Text.Encoding]::UTF8)
try {
    $zipPaths = [string[]]@($paths + 'manifest.json')
    [Array]::Sort($zipPaths, [StringComparer]::Ordinal)
    foreach ($relative in $zipPaths) {
        $entry = $zip.CreateEntry($relative, [IO.Compression.CompressionLevel]::Optimal)
        $entry.LastWriteTime = [DateTimeOffset]::new(2020,1,1,0,0,0,[TimeSpan]::Zero)
        $entry.ExternalAttributes = 0
        $input = [IO.File]::OpenRead((Get-ChildPath $packageRoot $relative))
        $entryStream = $entry.Open()
        try { $input.CopyTo($entryStream) } finally { $entryStream.Dispose(); $input.Dispose() }
    }
} finally { $zip.Dispose(); $stream.Dispose() }
$zipDigest = (Get-BetaFileHash -LiteralPath $zipTemp).Hash.ToLowerInvariant()
if (Test-Path -LiteralPath $zipPath) {
    if ((Get-BetaFileHash -LiteralPath $zipPath).Hash.ToLowerInvariant() -ne $zipDigest) { throw 'Existing ZIP differs. Compare builder/runtime versions before replacing it.' }
    Remove-Item -LiteralPath $zipTemp -Force
} else { [IO.File]::Move($zipTemp, $zipPath) }
Write-AtomicText ($zipPath + '.sha256') ($zipDigest + '  ' + [IO.Path]::GetFileName($zipPath) + "`n")
Write-Host "Package: $zipPath"
Write-Host "SHA256: $zipDigest"
Write-Host "Distribution ready: $redistributable; license review: $licenseComplete; pending acceptance: $($pendingGates -join ',')."
