# Renders assets/social-preview.png (1280x640, GitHub's social preview size) from assets/brand/GloriaSetup.png and the
# bundled fonts. The brand files are only read. Upload the result in the repo's Settings -> General -> Social preview.
#   powershell -ExecutionPolicy Bypass -File tools\make-social-preview.ps1
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$root = Resolve-Path "$PSScriptRoot\.."
$W = 1280; $H = 640

$fonts = New-Object System.Drawing.Text.PrivateFontCollection
foreach ($f in 'Archivo-800', 'Inter-500', 'Inter-600', 'JetBrainsMono-500') { $fonts.AddFontFile("$root\assets\fonts\$f.ttf") }
function Family($name) { $fonts.Families | Where-Object { $_.Name -eq $name } | Select-Object -First 1 }
# Each static TTF is its own family, so the weight is in the name and every font below uses the Regular style.
$archivo = Family 'Archivo ExtraBold'; $inter = Family 'Inter Medium'; $interSemi = Family 'Inter SemiBold'
$mono = Family 'JetBrains Mono Medium'
if (-not $interSemi) { throw 'Inter SemiBold not loaded' }
if (-not ($archivo -and $inter -and $mono)) { throw "fonts not loaded: $($fonts.Families.Name -join ', ')" }

$brand = [System.Drawing.Image]::FromFile("$root\assets\brand\GloriaSetup.png")
$green = ([System.Drawing.Bitmap]$brand).GetPixel(12, $brand.Height / 2)

$bmp = New-Object System.Drawing.Bitmap $W, $H
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.SmoothingMode = 'AntiAlias'; $g.InterpolationMode = 'HighQualityBicubic'; $g.TextRenderingHint = 'AntiAliasGridFit'
$hex = { param($h) [System.Drawing.ColorTranslator]::FromHtml($h) }
$g.Clear((& $hex '#0b0b0c'))

# Soft glow in the brand green behind the text, like the installer's accent glow.
$glow = New-Object System.Drawing.Drawing2D.GraphicsPath
$glow.AddEllipse(-260, -220, 1040, 900)
$pg = New-Object System.Drawing.Drawing2D.PathGradientBrush $glow
$pg.CenterColor = [System.Drawing.Color]::FromArgb(46, $green)
$pg.SurroundColors = @([System.Drawing.Color]::FromArgb(0, $green))
$g.FillPath($pg, $glow)

# Portrait on the right, full height, with a thin divider.
$g.DrawImage($brand, $W - $H, 0, $H, $H)
$g.FillRectangle((New-Object System.Drawing.SolidBrush $green), $W - $H - 4, 0, 4, $H)

$cream = New-Object System.Drawing.SolidBrush (& $hex '#f2f1ee')
$muted = New-Object System.Drawing.SolidBrush (& $hex '#a1a1aa')
$fmt = [System.Drawing.StringFormat]::GenericTypographic
$x = 72

$g.DrawString('projectGloria', (New-Object System.Drawing.Font $mono, 22, 'Regular', 'Pixel'), (New-Object System.Drawing.SolidBrush $green), $x, 150, $fmt)
$g.DrawString('GloriaSetup', (New-Object System.Drawing.Font $archivo, 92, 'Regular', 'Pixel'), $cream, ($x - 4), 184, $fmt)
$sub = New-Object System.Drawing.Font $inter, 27, 'Regular', 'Pixel'
$g.DrawString('A per-user Windows installer.', $sub, $muted, $x, 310, $fmt)
$g.DrawString('No admin. One small exe. Its own icon.', $sub, $muted, $x, 348, $fmt)

# Feature chips.
$chipFont = New-Object System.Drawing.Font $interSemi, 19, 'Regular', 'Pixel'
$chipBg = New-Object System.Drawing.SolidBrush (& $hex '#141418')
$chipLine = New-Object System.Drawing.Pen (& $hex '#26262d'), 1.5
$cx = $x; $cy = 446
foreach ($t in 'Rust', 'Direct2D', 'EN FR ES TR', 'GitHub Action') {
    $w = [math]::Ceiling($g.MeasureString($t, $chipFont, 1000, $fmt).Width) + 36
    $r = New-Object System.Drawing.Drawing2D.GraphicsPath
    $d = 40
    $r.AddArc($cx, $cy, $d, $d, 90, 180); $r.AddArc($cx + $w - $d, $cy, $d, $d, 270, 180); $r.CloseFigure()
    $g.FillPath($chipBg, $r); $g.DrawPath($chipLine, $r)
    $g.DrawString($t, $chipFont, $cream, ($cx + 18), ($cy + 9), $fmt)
    $cx += $w + 12
}

$out = "$root\assets\social-preview.png"
$bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose(); $brand.Dispose()
Write-Host "wrote $out"
