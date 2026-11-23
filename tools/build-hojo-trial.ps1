$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location $projectRoot
try {
    $model = Join-Path $projectRoot 'resources/hojo/Hojo-TTS-Light-40M-llm.onnx'
    $python = Join-Path $projectRoot '.build/hojo-venv/Scripts/python.exe'
    if (!(Test-Path $model)) { throw 'Hojo model is not installed under resources/hojo.' }
    if (!(Test-Path $python)) { throw 'Hojo CPU test runtime is not installed under .build/hojo-venv.' }
    cargo test --locked
    if ($LASTEXITCODE -ne 0) { throw 'Rust tests failed.' }
    cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'Rust release build failed.' }
    $metadata = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
    $executable = Join-Path $metadata.target_directory 'x86_64-pc-windows-msvc/release/AgentVoice.exe'
    $trial = Join-Path $projectRoot 'AgentVoice-Hojo-试用.exe'
    try {
        Copy-Item $executable $trial -Force
    } catch [System.IO.IOException] {
        if (!(Test-Path -LiteralPath $trial)) { throw }
        $previous = Join-Path $projectRoot "AgentVoice-Hojo-previous-$([Guid]::NewGuid().ToString('N')).exe"
        Move-Item -LiteralPath $trial -Destination $previous
        try {
            Copy-Item $executable $trial -Force
        } catch {
            Move-Item -LiteralPath $previous -Destination $trial -Force
            throw
        }
        Write-Host "The open trial window is using its previous executable: $previous"
    }
    Write-Host "Built local Hojo trial: $trial"
    Write-Host 'The normal AgentVoice.exe and portable release ZIP were not replaced.'
} finally { Pop-Location }
