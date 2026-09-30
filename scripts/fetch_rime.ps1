[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$taskRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$runtimeRoot = Join-Path $taskRoot 'target\rime-runtime'
New-Item -ItemType Directory -Force -Path $runtimeRoot | Out-Null
if (-not (Get-Command 7z -ErrorAction SilentlyContinue)) { throw 'Install 7-Zip and put 7z.exe on PATH.' }

function Get-PinnedFile([string]$Url, [string]$Path, [string]$Sha256) {
    if (Test-Path -LiteralPath $Path) {
        $digest = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
        if ($digest -eq $Sha256) { return }
    }
    $download = $Path + '.download'
    & curl.exe --fail --location --retry 3 --output $download $Url
    if ($LASTEXITCODE -ne 0) { throw "Download failed: $Url" }
    $digest = (Get-FileHash -LiteralPath $download -Algorithm SHA256).Hash
    if ($digest -ne $Sha256) { throw "SHA256 mismatch: $Url (got $digest)" }
    Move-Item -LiteralPath $download -Destination $Path -Force
}

Get-PinnedFile 'https://github.com/rime/librime/releases/download/1.17.0/rime-33e7814-Windows-msvc-x64.7z' (Join-Path $runtimeRoot 'librime.7z') '7478c7caa4ff6b37de86daba1f7ce4a994a4f5ba24872a820fb2b3a9b01fed15'
Get-PinnedFile 'https://github.com/rime/librime/releases/download/1.17.0/rime-deps-33e7814-Windows-msvc-x64.7z' (Join-Path $runtimeRoot 'deps.7z') '9ef5608d8a54ff52bbad7a9b4128de42b232f8e3dd1f5fd3bff42a0b1bacd7e8'
& 7z x (Join-Path $runtimeRoot 'librime.7z') "-o$(Join-Path $runtimeRoot 'native')" -y
if ($LASTEXITCODE -ne 0) { throw 'librime extraction failed' }
& 7z x (Join-Path $runtimeRoot 'deps.7z') "-o$(Join-Path $runtimeRoot 'deps')" 'share/opencc/*' -y
if ($LASTEXITCODE -ne 0) { throw 'OpenCC extraction failed' }
$shared = Join-Path $runtimeRoot 'shared'
New-Item -ItemType Directory -Force -Path $shared | Out-Null
Get-PinnedFile 'https://raw.githubusercontent.com/rime/rime-luna-pinyin/56b934b099dfbeab842320f13aa8b461a6ab3e42/luna_pinyin.dict.yaml' (Join-Path $shared 'luna_pinyin.dict.yaml') '75bcf6eb3ff62b129882ed89cc22b2d80a5347aa72bcfa2ccc839bac298e7314'
Get-PinnedFile 'https://raw.githubusercontent.com/rime/rime-essay/054920de4f54c9e5994276a96a4fc2a35cb51aa3/essay.txt' (Join-Path $shared 'essay.txt') '3e8512ebaa6657e35961b1f5762060717c647e3bf471d5e3b86e783b4e934ffb'
Copy-Item -LiteralPath (Join-Path $runtimeRoot 'deps\share\opencc') -Destination $shared -Recurse -Force
Copy-Item -LiteralPath (Join-Path $taskRoot 'config\rime\default.yaml'), (Join-Path $taskRoot 'config\rime\wufan_pinyin.schema.yaml') -Destination $shared -Force
Copy-Item -LiteralPath (Join-Path $taskRoot 'crates\rime-sys\vendor\LICENSE') -Destination (Join-Path $runtimeRoot 'LIBRIME-LICENSE') -Force
Write-Host "Pinned runtime prepared: $runtimeRoot"
Write-Host 'Run ime-broker --deploy once to compile the dictionary. Existing Rime user data is not used.'
