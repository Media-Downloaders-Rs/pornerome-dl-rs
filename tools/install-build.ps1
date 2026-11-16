param([switch]$Launch)
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot)).TrimEnd([IO.Path]::DirectorySeparatorChar)
$site = (Split-Path -Leaf $root) -replace '-dl-rs$',''
$name = "$site-dl"
$source = Join-Path $root "target/release/$name.exe"
$destination = Join-Path $root "$name.exe"
if (Get-Process -Name $name -ErrorAction SilentlyContinue) { throw "Cierra $name antes de actualizar." }
if (-not (Test-Path -LiteralPath $source)) { throw 'Compila primero con cargo build --release --offline --locked.' }
$temporary = Join-Path $root ('target/install-' + [Guid]::NewGuid().ToString('N') + '.exe')
[IO.File]::Copy($source,$temporary)
$expected = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
if ((Get-FileHash -LiteralPath $temporary -Algorithm SHA256).Hash -ne $expected) { throw 'La copia no coincide.' }
if (Test-Path -LiteralPath $destination) {
    $backupDirectory = Join-Path $root 'target/previous-builds'
    [IO.Directory]::CreateDirectory($backupDirectory) | Out-Null
    $backup = Join-Path $backupDirectory ($name + '-' + [Guid]::NewGuid().ToString('N') + '.exe')
    [IO.File]::Replace($temporary,$destination,$backup)
} else { [IO.File]::Move($temporary,$destination) }
if ((Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash -ne $expected) { throw 'El ejecutable instalado no coincide.' }
Write-Output "Installed $name.exe"
if ($Launch) { Start-Process -FilePath $destination -WorkingDirectory $root -WindowStyle Normal }