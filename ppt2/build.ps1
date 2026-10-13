# Builds Titan for PPT2 into ppt2\dist. Needs Rust and Visual Studio 2022 (MSBuild + .NET Framework 4.8 targeting pack).
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
$lib = Join-Path $here 'lib'
$dist = Join-Path $here 'dist'

# Native tools log progress to stderr, which Windows PowerShell 5.1 would turn into errors under 'Stop'.
function Invoke-Native($exe) {
    $ErrorActionPreference = 'Continue'
    & $exe @args 2>&1 | ForEach-Object { "$_" }
    if ($LASTEXITCODE -ne 0) { throw "$exe failed ($LASTEXITCODE)" }
}

# Binaries from the official Zetris release, kept out of the repo: the PPT2 memory reader (closed source),
# MisaMinoNET (our DLL speaks its interface), UI libraries and the virtual gamepad driver.
if (-not (Test-Path (Join-Path $lib 'Zetris.PPT.Memory.dll'))) {
    $zip = Join-Path $env:TEMP 'Zetris-38.zip'
    $unzipped = Join-Path $env:TEMP 'Zetris-38'
    Invoke-WebRequest 'https://github.com/ZetrisAI/Zetris/releases/download/38/Zetris-38.zip' -OutFile $zip
    Expand-Archive $zip -DestinationPath $unzipped -Force
    New-Item -ItemType Directory -Force $lib | Out-Null
    Copy-Item (Join-Path $unzipped 'Zetris-38\*') $lib -Recurse -Force -Exclude 'Zetris.PPT.exe', 'Zetris.dll', 'MisaMino.dll'
}

Invoke-Native cargo build --release --manifest-path (Join-Path $here '..\Cargo.toml')

$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$msbuild = & $vswhere -latest -find 'MSBuild\**\Bin\MSBuild.exe' | Select-Object -First 1
Invoke-Native $msbuild (Join-Path $here 'Titan.PPT2.csproj') /p:Configuration=Release /p:Platform=x64 /v:minimal /nologo

New-Item -ItemType Directory -Force $dist | Out-Null
Copy-Item (Join-Path $lib '*') $dist -Recurse -Force
Copy-Item (Join-Path $here 'bin\*') $dist -Recurse -Force
Copy-Item (Join-Path $here '..\target\release\tetris_ai.dll') (Join-Path $dist 'MisaMino.dll') -Force
Copy-Item (Join-Path $here 'LICENSE-Zetris') $dist -Force
Write-Host "Built $dist\TitanPPT2.exe"
