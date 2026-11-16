# Build the Windows release package of Snowballz into dist\.
#
#   powershell -ExecutionPolicy Bypass -File tools\make-release.ps1
#   -> dist\snowballz-<version>-windows-x64.zip
#
# SDL2.dll is taken from the project folder if it is there, otherwise the
# official SDL $SDL2_VERSION build is downloaded.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
Set-Location (Join-Path $PSScriptRoot '..')

$version = (Get-Content VERSION -Raw).Trim()
$sdlVersion = if ($env:SDL2_VERSION) { $env:SDL2_VERSION } else { '2.32.10' }

cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

$name = "snowballz-$version-windows-x64"
$out = Join-Path 'dist' $name
if (Test-Path $out) { Remove-Item -Recurse -Force $out }
New-Item -ItemType Directory -Force (Join-Path $out 'licenses') | Out-Null

Copy-Item 'target\release\snowballz.exe' $out
Copy-Item -Recurse 'data', 'maps', 'plugins' $out
Copy-Item 'icon.png', 'README.md', 'LICENSE' $out
Copy-Item 'tools\licenses\SDL2.txt' (Join-Path $out 'licenses')
(Get-Content 'tools\release\PLAY-windows.txt') -replace '@VERSION@', $version |
    Set-Content (Join-Path $out 'PLAY.txt')

if (Test-Path 'SDL2.dll') {
    Copy-Item 'SDL2.dll' $out
} else {
    $zip = "dist\SDL2-$sdlVersion-win32-x64.zip"
    if (-not (Test-Path $zip)) {
        $url = "https://github.com/libsdl-org/SDL/releases/download/release-$sdlVersion/SDL2-$sdlVersion-win32-x64.zip"
        Write-Host "Downloading $url"
        Invoke-WebRequest -Uri $url -OutFile $zip
    }
    $tmp = 'dist\sdl2-win'
    if (Test-Path $tmp) { Remove-Item -Recurse -Force $tmp }
    Expand-Archive -Force $zip $tmp
    $dll = Get-ChildItem -Recurse -Path $tmp -Filter 'SDL2.dll' | Select-Object -First 1
    if (-not $dll) { throw "no SDL2.dll in $zip" }
    Copy-Item $dll.FullName $out
}

# zip with "/" in the paths (Compress-Archive in Windows PowerShell 5 writes
# "\", which only Windows' own unzip understands)
Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
$dist = (Resolve-Path 'dist').Path
$zipOut = Join-Path $dist "$name.zip"
if (Test-Path $zipOut) { Remove-Item -Force $zipOut }
$zip = [System.IO.Compression.ZipFile]::Open($zipOut, 'Create')
try {
    Get-ChildItem -Recurse -File (Join-Path $dist $name) | ForEach-Object {
        $entry = $_.FullName.Substring($dist.Length + 1).Replace('\', '/')
        [System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip, $_.FullName, $entry, 'Optimal') | Out-Null
    }
} finally {
    $zip.Dispose()
}
Write-Host $zipOut
