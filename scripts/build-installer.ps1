param(
    [Parameter(Mandatory=$true)][string]$Compiler,
    [Parameter(Mandatory=$true)][string]$OutputDirectory,
    [string]$Python = 'python'
)
$ErrorActionPreference = 'Stop'
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$output = [IO.Path]::GetFullPath($OutputDirectory)
$staging = Join-Path $repo ('target\installer-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $staging -Force | Out-Null
New-Item -ItemType Directory -Path $output -Force | Out-Null
$previousFlags = $env:CARGO_ENCODED_RUSTFLAGS
$previousCflags = $env:CFLAGS
Push-Location $repo
try {
    # Explicit target keeps host procedural macros independent of static-CRT flags.
    $env:CARGO_ENCODED_RUSTFLAGS = @(
        '-Ctarget-feature=+crt-static',
        ('--remap-path-prefix=' + $repo + '=/app'),
        ('--remap-path-prefix=' + $env:USERPROFILE + '=/build'),
        ('--remap-path-prefix=' + $env:TEMP + '=/tmp')
    ) -join [char]31
    $env:CFLAGS = '/pathmap:' + $env:USERPROFILE + '=/build'
    cargo rustc --profile dist --locked --target x86_64-pc-windows-msvc --bin ndpiviewer-rs -- -C link-arg=/DEBUG:NONE
    if ($LASTEXITCODE -ne 0) { throw 'Distribution build failed.' }
    Copy-Item -LiteralPath 'target\x86_64-pc-windows-msvc\dist\ndpiviewer-rs.exe' -Destination (Join-Path $staging 'NDPIViewer.exe')
    Copy-Item -LiteralPath (Join-Path $repo 'LICENSE') -Destination (Join-Path $staging 'LICENSE.txt')

    $metadataText = cargo metadata --locked --format-version 1 --filter-platform x86_64-pc-windows-msvc --offline
    if ($LASTEXITCODE -ne 0) { throw 'Dependency metadata failed.' }
    $metadata = ($metadataText -join "`n") | ConvertFrom-Json
    $root = $metadata.packages | Where-Object { $_.id -eq $metadata.resolve.root }
    $nodes = @{}
    foreach ($node in $metadata.resolve.nodes) { $nodes[$node.id] = $node }
    $seen = [Collections.Generic.HashSet[string]]::new()
    $pending = [Collections.Generic.Queue[string]]::new()
    $pending.Enqueue($root.id)
    while ($pending.Count -gt 0) {
        $id = $pending.Dequeue()
        if (!$seen.Add($id)) { continue }
        foreach ($dependency in $nodes[$id].deps) {
            if (@($dependency.dep_kinds | Where-Object { $_.kind -ne 'dev' }).Count -gt 0) {
                $pending.Enqueue($dependency.pkg)
            }
        }
    }
    $notices = [Text.StringBuilder]::new()
    [void]$notices.AppendLine('NDPI Viewer - third-party notices')
    [void]$notices.AppendLine("NDPI Viewer $($root.version) is licensed under GPL-3.0-or-later. See LICENSE.txt.")
    [void]$notices.AppendLine("Corresponding application source: https://github.com/Mancuerna/ndpiviewer-rs/tree/v$($root.version)")
    [void]$notices.AppendLine('Dependency license identifiers and included upstream license texts follow.')
    foreach ($package in ($metadata.packages | Where-Object { $_.id -ne $root.id -and $seen.Contains($_.id) } | Sort-Object name,version)) {
        [void]$notices.AppendLine("`n=== $($package.name) $($package.version) ===")
        [void]$notices.AppendLine("License: $($package.license)")
        [void]$notices.AppendLine("Source: $($package.repository)")
        $packageDir = Split-Path $package.manifest_path
        $licenseFiles = @(Get-ChildItem -LiteralPath $packageDir -File | Where-Object { $_.Name -match '^(LICENSE|COPYING|NOTICE|COPYRIGHT)' })
        if ($package.license_file) {
            $licenseFiles += Get-Item -LiteralPath (Join-Path $packageDir $package.license_file)
        }
        foreach ($folder in @('licenses','LICENSES')) {
            $licenseDir = Join-Path $packageDir $folder
            if (Test-Path -LiteralPath $licenseDir -PathType Container) { $licenseFiles += Get-ChildItem -LiteralPath $licenseDir -File -Recurse }
        }
        foreach ($license in ($licenseFiles | Sort-Object FullName -Unique)) {
            [void]$notices.AppendLine([IO.File]::ReadAllText($license.FullName))
        }
    }
    [IO.File]::WriteAllText((Join-Path $staging 'ThirdPartyNotices.txt'), $notices.ToString(), [Text.UTF8Encoding]::new($false))

    # Fail closed on extra payloads or embedded private build paths, including UTF-16.
    $allowed = @('NDPIViewer.exe','ThirdPartyNotices.txt','LICENSE.txt')
    foreach ($file in Get-ChildItem -LiteralPath $staging -File) {
        if ($file.Name -notin $allowed) { throw 'Unexpected payload file.' }
        $bytes = [IO.File]::ReadAllBytes($file.FullName)
        foreach ($encoding in @([Text.Encoding]::UTF8, [Text.Encoding]::Unicode)) {
            $text = $encoding.GetString($bytes)
            foreach ($private in @($env:USERPROFILE, $repo, 'example_images', 'JNOC-GC2003', 'Hamamatsu_fluorescence_Manuel_Stritt')) {
                if ($text.IndexOf($private, [StringComparison]::OrdinalIgnoreCase) -ge 0) { throw "Private build data detected in $($file.Name)." }
            }
            if ($text -match '(?i)[a-z]:[\\/]Users[\\/]') { throw "Local user path detected in $($file.Name)." }
        }
    }
    & $Python (Join-Path $repo 'scripts\audit-windows.py') (Join-Path $staging 'NDPIViewer.exe')
    if ($LASTEXITCODE -ne 0) { throw 'Executable dependency/privacy audit failed.' }
    & $Compiler "/DPayloadDir=$staging" "/DOutputDir=$output" "/DAppVersion=$($root.version)" (Join-Path $repo 'installer\windows.iss')
    if ($LASTEXITCODE -ne 0) { throw 'Installer compilation failed.' }
    $installer = Join-Path $output "NDPI-Viewer-$($root.version)-Windows-x64-Setup.exe"
    $hash = (Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash
    [IO.File]::WriteAllText(($installer + '.sha256'), "$hash  $([IO.Path]::GetFileName($installer))`n")
    Write-Output "Installer: $installer"
    Write-Output "Verified payload: $staging"
} finally {
    $env:CARGO_ENCODED_RUSTFLAGS = $previousFlags
    $env:CFLAGS = $previousCflags
    Pop-Location
}
