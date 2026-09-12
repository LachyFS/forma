# Build a portable host-architecture Windows directory and zip.
param([ValidateSet("debug", "release")][string]$Profile = "release")
$ErrorActionPreference = "Stop"
if ($env:OS -ne "Windows_NT") { throw "Build the Windows bundle on Windows." }
$repository = Split-Path -Parent $PSScriptRoot
Push-Location $repository
try {
    $buildArguments = @("build", "--locked", "-p", "forma", "--target-dir", (Join-Path $repository "target"))
    if ($Profile -eq "release") { $buildArguments += "--release" }
    & cargo @buildArguments
    if ($LASTEXITCODE -ne 0) { throw "Cargo build failed with exit code $LASTEXITCODE" }
    $output = Join-Path $repository "target/$Profile"
    $architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant()
    $bundle = Join-Path $output "forma-windows-$architecture"
    if ((Test-Path $bundle) -and ((Get-Item $bundle).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "Refusing linked bundle output: $bundle"
    }
    New-Item -ItemType Directory -Force -Path $bundle | Out-Null
    Copy-Item (Join-Path $output "forma.exe") $bundle
    & python scripts/setup-denoiser.py --output-dir (Join-Path $bundle "oidn")
    if ($LASTEXITCODE -ne 0) { throw "OIDN runtime packaging failed" }
    Copy-Item (Join-Path $repository "README.md") $bundle
    Compress-Archive -Path $bundle -DestinationPath "$bundle.zip" -Force
    Write-Output "Bundle: $bundle.zip"
    Write-Output "Run: $bundle/forma.exe"
} finally {
    Pop-Location
}
