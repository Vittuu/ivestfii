#requires -Version 7.0
[CmdletBinding()]
param(
    [string]$SigningKeyPath = '',
    [string]$OutputDirectory = 'dist',
    [string]$ExpectedVersion = '',
    [string]$ReleaseNotes = 'Atualização do IvestFII para Windows.',
    [switch]$ValidateOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Get-SectionVersion([string]$Path, [string]$Section) {
    $source = [IO.File]::ReadAllText($Path)
    $sectionPattern = '(?ms)^\[' + [regex]::Escape($Section) + '\]\s*(?<body>.*?)(?=^\[|\z)'
    $sectionMatch = [regex]::Match($source, $sectionPattern)
    $versionMatch = [regex]::Match($sectionMatch.Groups['body'].Value, '(?m)^version\s*=\s*"([^"]+)"\s*$')
    if (-not $sectionMatch.Success -or -not $versionMatch.Success) {
        throw "Versao explicita nao encontrada em $Path [$Section]."
    }
    return $versionMatch.Groups[1].Value
}

function Write-Utf8([string]$Path, [string]$Content) {
    [IO.File]::WriteAllText($Path, $Content + "`n", [Text.UTF8Encoding]::new($false))
}

$projectDirectory = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$config = Get-Content -LiteralPath (Join-Path $projectDirectory 'src-tauri/tauri.conf.json') -Raw | ConvertFrom-Json
$package = Get-Content -LiteralPath (Join-Path $projectDirectory 'package.json') -Raw | ConvertFrom-Json
$packageLock = Get-Content -LiteralPath (Join-Path $projectDirectory 'package-lock.json') -Raw | ConvertFrom-Json -AsHashtable
$version = [string]$config.version
$semver = '^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$'
if ($version -notmatch $semver) { throw 'tauri.conf.json deve conter uma versao SemVer explicita.' }
$versions = @(
    [string]$package.version,
    [string]$packageLock['version'],
    [string]$packageLock['packages']['']['version'],
    (Get-SectionVersion (Join-Path $projectDirectory 'src-tauri/Cargo.toml') 'package'),
    (Get-SectionVersion (Join-Path $projectDirectory 'Cargo.toml') 'workspace.package')
)
if (@($versions | Where-Object { $_ -ne $version }).Count -gt 0) {
    throw 'Alinhe as versoes de package.json, package-lock.json, Cargo.toml e src-tauri antes de gerar a distribuicao.'
}
if ($ExpectedVersion -and $version -ne $ExpectedVersion.TrimStart('v')) {
    throw 'A versao do codigo nao corresponde a tag de release solicitada.'
}
if ($config.identifier -ne 'com.ivestfii.desktop') { throw 'O identifier mudou; isso separaria os dados da carteira existente.' }
if ($config.bundle.createUpdaterArtifacts -ne $true) { throw 'Ative bundle.createUpdaterArtifacts para gerar assinaturas de atualizacao.' }
if ($config.bundle.windows.nsis.installMode -ne 'currentUser') { throw 'O instalador deve usar installMode currentUser.' }
if ($config.bundle.windows.allowDowngrades -ne $false) { throw 'Desative allowDowngrades na distribuicao.' }
if ($config.bundle.windows.webviewInstallMode.type -ne 'downloadBootstrapper') {
    throw 'O instalador unico deve instalar o WebView2 com downloadBootstrapper.'
}
$endpoint = 'https://github.com/Vittuu/ivestfii/releases/latest/download/latest.json'
if (@($config.plugins.updater.endpoints) -notcontains $endpoint -or -not $config.plugins.updater.pubkey) {
    throw 'Configure a chave publica e o endpoint oficial do atualizador antes de distribuir.'
}
if ($env:OS -ne 'Windows_NT') { throw 'Execute este script no Windows com Rust MSVC e WebView2.' }
$hostInformation = & rustc -vV
if ($LASTEXITCODE -ne 0 -or @($hostInformation | Where-Object { $_ -eq 'host: x86_64-pc-windows-msvc' }).Count -ne 1) {
    throw 'Esta distribuicao suporta Windows x64 com o toolchain x86_64-pc-windows-msvc.'
}

$originalKey = $env:TAURI_SIGNING_PRIVATE_KEY
$originalPassword = $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD
try {
    if ($SigningKeyPath -or -not $env:TAURI_SIGNING_PRIVATE_KEY) {
        if (-not $SigningKeyPath) {
            if (-not $env:LOCALAPPDATA) { throw 'LOCALAPPDATA indisponivel; informe SigningKeyPath.' }
            $SigningKeyPath = Join-Path $env:LOCALAPPDATA 'IvestFII/release-keys/updater.key'
        }
        $keyFile = Get-Item -LiteralPath $SigningKeyPath
        if ($keyFile.PSIsContainer -or $keyFile.Length -eq 0) { throw 'O arquivo da chave de assinatura esta vazio ou invalido.' }
        if ($keyFile.FullName.StartsWith($projectDirectory + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
            throw 'Mantenha a chave privada fora do repositorio.'
        }
        $env:TAURI_SIGNING_PRIVATE_KEY = $keyFile.FullName
    }
    # A chave pode vir como conteudo de um secret do CI. Nunca a escreva em logs ou artefatos.
    if ($null -eq $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD) { $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = '' }
    if ($ValidateOnly) {
        Write-Output "Configuracao de distribuicao Windows x64 $version validada. Nenhum build ou envio executado."
        return
    }

    if (-not [IO.Path]::IsPathRooted($OutputDirectory)) { $OutputDirectory = Join-Path $projectDirectory $OutputDirectory }
    $outputPath = [IO.Path]::GetFullPath($OutputDirectory)
    $metadataPath = Join-Path $outputPath 'release'
    [void][IO.Directory]::CreateDirectory($metadataPath)
    $buildStarted = [DateTime]::UtcNow.AddSeconds(-5)
    Push-Location $projectDirectory
    try {
        & npm.cmd run tauri -- build --ci --bundles nsis -- --locked
        if ($LASTEXITCODE -ne 0) { throw 'Falha ao compilar ou assinar o instalador Tauri.' }
    }
    finally { Pop-Location }

    $bundleDirectory = Join-Path $projectDirectory 'target/release/bundle/nsis'
    $installers = @(Get-ChildItem -LiteralPath $bundleDirectory -File | Where-Object {
        $_.Name -like "*_${version}_x64-setup.exe" -and $_.LastWriteTimeUtc -ge $buildStarted
    })
    if ($installers.Count -ne 1) { throw 'O build nao produziu exatamente um instalador NSIS x64 novo para esta versao.' }
    $installer = $installers[0]
    $signaturePath = $installer.FullName + '.sig'
    if (-not (Test-Path -LiteralPath $signaturePath -PathType Leaf)) { throw 'A assinatura Tauri .sig nao foi gerada; o instalador nao sera distribuido.' }
    $signature = [IO.File]::ReadAllText($signaturePath).Trim()
    try { $signatureBytes = [Convert]::FromBase64String($signature) }
    catch { throw 'A assinatura Tauri gerada nao esta em Base64 valido.' }
    if ($signatureBytes.Length -lt 64) { throw 'A assinatura Tauri gerada esta incompleta.' }

    $installerName = 'IvestFII-Setup.exe'
    $destination = Join-Path $outputPath $installerName
    Copy-Item -LiteralPath $installer.FullName -Destination $destination -Force
    Write-Utf8 (Join-Path $metadataPath ($installerName + '.sig')) $signature
    $manifest = [ordered]@{
        version = $version
        notes = $ReleaseNotes
        pub_date = [DateTimeOffset]::UtcNow.ToString('o')
        platforms = [ordered]@{
            'windows-x86_64' = [ordered]@{
                signature = $signature
                url = "https://github.com/Vittuu/ivestfii/releases/download/v$version/$installerName"
            }
        }
    }
    Write-Utf8 (Join-Path $metadataPath 'latest.json') ($manifest | ConvertTo-Json -Depth 6)
    $checksum = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant()
    Write-Utf8 (Join-Path $metadataPath 'SHA256SUMS.txt') "$checksum  $installerName"
    Push-Location $projectDirectory
    try {
        & cargo run --quiet -p ivestfii-desktop --example verify-installer --locked --offline -- $destination (Join-Path $metadataPath ($installerName + '.sig')) (Join-Path $metadataPath 'latest.json')
        if ($LASTEXITCODE -ne 0) { throw 'A verificacao offline de assinatura e integridade falhou; nao distribua estes artefatos.' }
    }
    finally { Pop-Location }
    Write-Output "Instalador unico: $destination"
    Write-Output "Metadados de atualizacao assinada: $metadataPath"
    Write-Output 'Nenhuma release foi publicada. Os dados da carteira e as chaves privadas nao fazem parte dos artefatos.'
}
finally {
    $env:TAURI_SIGNING_PRIVATE_KEY = $originalKey
    $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $originalPassword
}
