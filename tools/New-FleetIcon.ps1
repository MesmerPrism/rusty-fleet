[CmdletBinding()]
param(
    [switch]$Write
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$sourcePath = Join-Path $repoRoot 'assets\branding\rusty-fleet.svg'
$iconPath = Join-Path $repoRoot (
    'apps\fleet-console-wpf\Assets\rusty-fleet.ico')
$expectedSourceSha256 =
    '1dedfecaef954dda9bb6f4f133376535e4799908441e7832558a1f70f4ed6f79'
Import-Module (Join-Path $PSScriptRoot 'FleetIconProvenance.psm1') -Force

if (-not (Test-Path -LiteralPath $sourcePath -PathType Leaf)) {
    throw 'The canonical Fleet SVG source is missing.'
}

$source = ConvertTo-FleetCanonicalText -Text (
    [IO.File]::ReadAllText($sourcePath))
$sourceSha256 = Get-FleetCanonicalTextSha256 -Text $source
if ($sourceSha256 -cne $expectedSourceSha256) {
    throw (
        'The canonical Fleet SVG changed without updating its deterministic ' +
        'ICO generator binding.')
}
foreach ($marker in @(
    'viewBox="0 0 256 256"',
    'fill="#17324d"',
    'stroke="#72c7d4"',
    'fill="#f1a45b"')) {
    if (-not $source.Contains($marker, [StringComparison]::Ordinal)) {
        throw "The canonical Fleet SVG source is missing marker $marker."
    }
}

function Get-Channel {
    param(
        [double]$Coverage,
        [int]$Foreground,
        [int]$Background
    )
    return [byte][Math]::Round(
        ($Coverage * $Foreground) + ((1.0 - $Coverage) * $Background),
        [MidpointRounding]::AwayFromZero)
}

function Get-Pixel {
    param(
        [double]$X,
        [double]$Y
    )

    $background = @(0x17, 0x32, 0x4d)
    $teal = @(0x72, 0xc7, 0xd4)
    $orange = @(0xf1, 0xa4, 0x5b)
    $white = @(0xff, 0xff, 0xff)
    $color = $background

    $line = (
        (($X -ge 72) -and ($X -le 184) -and
         ([Math]::Abs($Y - 82) -le 8 -or
          [Math]::Abs($Y - 174) -le 8)) -or
        (($Y -ge 72) -and ($Y -le 184) -and
         ([Math]::Abs($X - 82) -le 8 -or
          [Math]::Abs($X - 174) -le 8))
    )
    if ($line) {
        $color = $teal
    }

    foreach ($center in @(@(72, 72), @(184, 72), @(72, 184), @(184, 184))) {
        $distance = [Math]::Sqrt(
            [Math]::Pow($X - $center[0], 2) +
            [Math]::Pow($Y - $center[1], 2))
        if ($distance -le 35) {
            $color = $white
        }
        if ($distance -le 25) {
            $color = $orange
        }
    }
    return $color
}

function New-Dib {
    param([int]$Size)

    $stride = $Size * 4
    $maskStride = [int]([Math]::Ceiling($Size / 32.0) * 4)
    $pixelBytes = $stride * $Size
    $maskBytes = $maskStride * $Size
    $stream = [IO.MemoryStream]::new()
    $writer = [IO.BinaryWriter]::new($stream)
    try {
        $writer.Write([int]40)
        $writer.Write([int]$Size)
        $writer.Write([int]($Size * 2))
        $writer.Write([int16]1)
        $writer.Write([int16]32)
        $writer.Write([int]0)
        $writer.Write([int]$pixelBytes)
        $writer.Write([int]0)
        $writer.Write([int]0)
        $writer.Write([int]0)
        $writer.Write([int]0)
        for ($row = $Size - 1; $row -ge 0; $row--) {
            for ($column = 0; $column -lt $Size; $column++) {
                $scale = 256.0 / $Size
                $samples = @()
                foreach ($offsetY in @(0.25, 0.75)) {
                    foreach ($offsetX in @(0.25, 0.75)) {
                        $samples += ,(Get-Pixel `
                            -X (($column + $offsetX) * $scale) `
                            -Y (($row + $offsetY) * $scale))
                    }
                }
                foreach ($channel in @(2, 1, 0)) {
                    $average = [Math]::Round(
                        ($samples[0][$channel] +
                         $samples[1][$channel] +
                         $samples[2][$channel] +
                         $samples[3][$channel]) / 4.0,
                        [MidpointRounding]::AwayFromZero)
                    $writer.Write([byte]$average)
                }
                $writer.Write([byte]0xff)
            }
        }
        $writer.Write([byte[]]::new($maskBytes))
        return $stream.ToArray()
    }
    finally {
        $writer.Dispose()
        $stream.Dispose()
    }
}

function New-Ico {
    $sizes = @(16, 32, 48, 256)
    $images = @($sizes | ForEach-Object { ,(New-Dib -Size $_) })
    $headerSize = 6 + (16 * $sizes.Count)
    $offset = $headerSize
    $stream = [IO.MemoryStream]::new()
    $writer = [IO.BinaryWriter]::new($stream)
    try {
        $writer.Write([int16]0)
        $writer.Write([int16]1)
        $writer.Write([int16]$sizes.Count)
        for ($index = 0; $index -lt $sizes.Count; $index++) {
            $size = $sizes[$index]
            $writer.Write([byte]$(if ($size -eq 256) { 0 } else { $size }))
            $writer.Write([byte]$(if ($size -eq 256) { 0 } else { $size }))
            $writer.Write([byte]0)
            $writer.Write([byte]0)
            $writer.Write([int16]1)
            $writer.Write([int16]32)
            $writer.Write([int]$images[$index].Length)
            $writer.Write([int]$offset)
            $offset += $images[$index].Length
        }
        foreach ($image in $images) {
            $writer.Write([byte[]]$image)
        }
        return $stream.ToArray()
    }
    finally {
        $writer.Dispose()
        $stream.Dispose()
    }
}

$expected = New-Ico
if ($Write) {
    $directory = Split-Path -Parent $iconPath
    [IO.Directory]::CreateDirectory($directory) | Out-Null
    [IO.File]::WriteAllBytes($iconPath, $expected)
}
if (-not (Test-Path -LiteralPath $iconPath -PathType Leaf)) {
    throw 'The generated Fleet ICO is missing. Run tools\New-FleetIcon.ps1 -Write.'
}
$actual = [IO.File]::ReadAllBytes($iconPath)
if (-not [Convert]::ToHexString($actual).Equals(
        [Convert]::ToHexString($expected),
        [StringComparison]::Ordinal)) {
    throw 'The generated Fleet ICO is stale. Run tools\New-FleetIcon.ps1 -Write.'
}

[ordered]@{
    schema = 'rusty.fleet.icon_validation.v1'
    source = 'assets/branding/rusty-fleet.svg'
    icon = 'apps/fleet-console-wpf/Assets/rusty-fleet.ico'
    sizes = @(16, 32, 48, 256)
    source_sha256 = $sourceSha256
    icon_sha256 = (
        Get-FileHash -LiteralPath $iconPath -Algorithm SHA256
    ).Hash.ToLowerInvariant()
    result = 'pass'
} | ConvertTo-Json -Compress
