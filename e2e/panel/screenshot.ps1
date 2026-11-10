# Saves a picture of the whole Windows desktop to the path given, so the platforms workflow can keep what
# the panel looked like. CaptureBlt takes in see-through windows, which a plain copy leaves out.
param([Parameter(Mandatory)][string]$Path)
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
$screen = [System.Windows.Forms.SystemInformation]::VirtualScreen
$image = New-Object System.Drawing.Bitmap $screen.Width, $screen.Height
$copy = [System.Drawing.CopyPixelOperation]::SourceCopy -bor [System.Drawing.CopyPixelOperation]::CaptureBlt
[System.Drawing.Graphics]::FromImage($image).CopyFromScreen($screen.Location, [System.Drawing.Point]::Empty, $screen.Size, $copy)
$image.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
