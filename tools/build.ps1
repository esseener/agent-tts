$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location $projectRoot
try {
    $localSource = Join-Path $projectRoot 'resources/kokoro'
    $notificationSource = Join-Path $projectRoot 'resources/notifications'
    if (!(Test-Path (Join-Path $notificationSource 'manifest.json'))) { throw 'Run python tools/prepare_notification_audio.py before packaging.' }
    foreach ($required in @('model.int8.onnx', 'voices.bin', 'lexicon-zh.txt', 'lexicon-us-en.txt', 'tokens.txt', 'sherpa-onnx-c-api.dll', 'onnxruntime.dll', 'manifest.json')) {
        if (!(Test-Path (Join-Path $localSource $required))) { throw "Local voice resources missing ($required). Run python tools/prepare_kokoro.py first." }
    }
    cargo test --locked
    if ($LASTEXITCODE -ne 0) { throw 'Rust tests failed.' }
    cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'Rust release build failed.' }
    $metadata = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
    $version = $metadata.packages[0].version
    $executable = Join-Path $metadata.target_directory 'x86_64-pc-windows-msvc/release/AgentVoice.exe'
    $release = Join-Path $projectRoot "dist/AgentVoice-$version"
    $oldExecutable = Join-Path $projectRoot 'AgentVoice.exe'
    if (Test-Path $oldExecutable) {
        $archiveDirectory = Join-Path $projectRoot 'archive/releases'
        New-Item -ItemType Directory -Force $archiveDirectory | Out-Null
        $sha = [System.Security.Cryptography.SHA256]::Create()
        $stream = [System.IO.File]::OpenRead($oldExecutable)
        try {
            $oldHash = [System.BitConverter]::ToString($sha.ComputeHash($stream)).Replace('-', '').ToLowerInvariant().Substring(0, 12)
        } finally {
            $stream.Dispose()
            $sha.Dispose()
        }
        $backupPath = Join-Path $archiveDirectory "AgentVoice-$oldHash.exe"
        if (!(Test-Path $backupPath)) { Copy-Item $oldExecutable $backupPath }
        $oldZip=Join-Path $projectRoot "dist/AgentVoice-$version.zip"
        if (Test-Path $oldZip) {
            $zipBackup=Join-Path $archiveDirectory "AgentVoice-$version-$oldHash.zip"
            if (!(Test-Path $zipBackup)) { Copy-Item $oldZip $zipBackup }
        }
    }
    New-Item -ItemType Directory -Force $release | Out-Null
    Copy-Item $executable (Join-Path $release 'AgentVoice.exe') -Force
    foreach ($name in @('启动.cmd', '使用说明.md')) { Copy-Item (Join-Path $projectRoot $name) $release -Force }
    $pluginSource = Join-Path $projectRoot 'plugins/agent-voice'
    $pluginDestination = Join-Path $release 'plugins/agent-voice'
    New-Item -ItemType Directory -Force $pluginDestination | Out-Null
    foreach ($name in @('orca-plugin.json', 'README.md', 'tsconfig.json')) {
        Copy-Item (Join-Path $pluginSource $name) $pluginDestination -Force
    }
    Copy-Item (Join-Path $pluginSource 'src') $pluginDestination -Recurse -Force
    $localDestination = Join-Path $release 'resources/kokoro'
    New-Item -ItemType Directory -Force $localDestination | Out-Null
    $manifest = Get-Content (Join-Path $localSource 'manifest.json') -Raw | ConvertFrom-Json
    $resourceNames = @($manifest.files.PSObject.Properties.Name) + @('manifest.json')
    foreach ($name in $resourceNames) {
        $target = Join-Path $localDestination $name
        New-Item -ItemType Directory -Force (Split-Path -Parent $target) | Out-Null
        Copy-Item (Join-Path $localSource $name) $target -Force
    }
    $notificationDestination = Join-Path $release 'resources/notifications'
    New-Item -ItemType Directory -Force $notificationDestination | Out-Null
    Get-ChildItem $notificationSource -File | ForEach-Object { Copy-Item $_.FullName $notificationDestination -Force }
    $voiceLibrary = Join-Path $release 'VoiceLibrary'
    New-Item -ItemType Directory -Force $voiceLibrary | Out-Null
    Copy-Item (Join-Path $projectRoot 'assets/kokoro-voice-catalog.json') (Join-Path $voiceLibrary 'voice-catalog.json') -Force
    Copy-Item (Join-Path $projectRoot 'docs/KOKORO_VOICE.md') (Join-Path $voiceLibrary '调音说明.md') -Force
    $listeningPage = Join-Path $projectRoot "docs/voice-samples-$version/试听对比.html"
    if (!(Test-Path $listeningPage)) { $listeningPage = Join-Path $projectRoot 'docs/voice-samples-0.4.0/试听对比.html' }
    if (Test-Path $listeningPage) { Copy-Item $listeningPage $voiceLibrary -Force }
    $latencyReport = Join-Path $projectRoot "docs/notification-latency-$version.json"
    if (Test-Path $latencyReport) { Copy-Item $latencyReport $voiceLibrary -Force }
    & $executable --export-tunings (Join-Path $voiceLibrary '调音预设.json')
    if ($LASTEXITCODE -ne 0) { throw 'Voice tuning export failed.' }
    $logoDirectory = Join-Path $release 'Logo'
    New-Item -ItemType Directory -Force $logoDirectory | Out-Null
    foreach ($name in @('logo-white.png', 'logo.png', 'logo-compact-white.png', 'logo-horizontal-white.png', 'logo-stacked-white.png', 'wordmark-white.png', 'AgentVoice.ico', 'README.md', 'branding.json')) {
        Copy-Item (Join-Path $projectRoot "assets/$name") $logoDirectory -Force
    }
    $zipPath = Join-Path $projectRoot "dist/AgentVoice-$version.zip"
    python tools/package_release.py --stage $release --exe $executable --out $zipPath
    if ($LASTEXITCODE -ne 0) { throw 'Portable package verification failed.' }
    try { Copy-Item $executable $oldExecutable -Force } catch {
        # Windows allows renaming an image that is still running. Keep that
        # process and its executable intact, then install the new entry point.
        # Keep its adjacent resource directory available until the user closes
        # the previous window; the recoverable release is already archived.
        $runningBackup=Join-Path $projectRoot "AgentVoice-previous-$oldHash.exe"
        if (Test-Path $runningBackup) { $runningBackup=Join-Path $projectRoot "AgentVoice-previous-$oldHash-$([DateTime]::UtcNow.Ticks).exe" }
        Move-Item $oldExecutable $runningBackup
        Copy-Item $executable $oldExecutable
        Write-Host "Previous app is still running; its executable is retained at $runningBackup"
    }
    if (!('AgentVoiceShellRefresh' -as [type])) {
        Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class AgentVoiceShellRefresh {
    [DllImport("shell32.dll", CharSet = CharSet.Unicode)]
    public static extern void SHChangeNotify(uint action, uint flags, string item, IntPtr unused);
}
'@
    }
    [AgentVoiceShellRefresh]::SHChangeNotify(0x2000, 0x1005, $oldExecutable, [IntPtr]::Zero)
    Write-Host "Built Agent Voice $version (Rust): $((Get-Item $executable).Length) bytes"
    Write-Host "Executable: $oldExecutable"
    Write-Host "Portable ZIP: $zipPath"
} finally { Pop-Location }
