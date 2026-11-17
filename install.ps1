# Install (or update) Orbital on Windows 10/11 from its latest GitHub release.
#
#   irm https://raw.githubusercontent.com/SirojAbdumavlonov/orbital/main/install.ps1 | iex
#
# Why a script: a browser marks what it downloads as "from the internet"
# (Mark of the Web), and SmartScreen then warns about an installer that isn't
# code-signed. A download made by PowerShell carries no such mark. The script
# only fetches the installer from this repository's releases and checks it
# against the SHA-256 checksum GitHub publishes before running it.
#
# The installer needs no admin rights: Orbital goes into your user profile.
#
# Settings (environment variables):
#   $env:ORBITAL_VERSION = "1.0.1"   a specific release instead of the latest
#   $env:ORBITAL_INTERACTIVE = "1"   show the installer's window instead of a silent install

# Everything runs inside a script block, so nothing here changes the
# settings of the PowerShell window it was pasted into.
& {
  $ErrorActionPreference = "Stop"
  $ProgressPreference = "SilentlyContinue"   # Invoke-WebRequest is far slower with it on
  [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

  $repo = "SirojAbdumavlonov/orbital"
  function Say($msg) { Write-Host "==> $msg" -ForegroundColor Cyan }
  function Fail($msg) { Write-Host "error: $msg" -ForegroundColor Red; throw $msg }

  $api = if ($env:ORBITAL_VERSION) { "https://api.github.com/repos/$repo/releases/tags/v$($env:ORBITAL_VERSION.TrimStart('v'))" }
         else { "https://api.github.com/repos/$repo/releases/latest" }
  try {
    $release = Invoke-RestMethod -Uri $api -Headers @{ Accept = "application/vnd.github+json"; "User-Agent" = "orbital-install" }
  } catch { Fail "couldn't read the release from GitHub ($api): $($_.Exception.Message)" }
  $version = $release.tag_name.TrimStart("v")
  Say "Orbital $version"

  if (-not [Environment]::Is64BitOperatingSystem) { Fail "Orbital for Windows is built for 64-bit Windows only" }
  $asset = $release.assets | Where-Object { $_.name -match '^Orbital_.*_x64-setup\.exe$' } | Select-Object -First 1
  if (-not $asset) { Fail "Orbital $version has no Windows installer" }

  $dir = Join-Path ([IO.Path]::GetTempPath()) ("orbital-" + [Guid]::NewGuid().ToString("N"))
  New-Item -ItemType Directory -Path $dir | Out-Null
  $file = Join-Path $dir $asset.name
  try {
    Say "Downloading $($asset.name)"
    Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $file -UseBasicParsing -Headers @{ "User-Agent" = "orbital-install" }

    if ($asset.digest -and $asset.digest.StartsWith("sha256:")) {
      $want = $asset.digest.Substring(7).ToLowerInvariant()
      $got = (Get-FileHash -Algorithm SHA256 -Path $file).Hash.ToLowerInvariant()
      if ($got -ne $want) { Fail "checksum mismatch for $($asset.name): the download is damaged or was changed" }
      Say "Checksum verified"
    }
    # Nothing set the mark, but remove it in case a security tool did.
    Unblock-File -Path $file

    $running = Get-Process -Name "orbital" -ErrorAction SilentlyContinue
    if ($running) {
      Say "Closing the running Orbital"
      $running | Stop-Process -Force
      Start-Sleep -Seconds 2
    }

    Say "Installing"
    $p = if ($env:ORBITAL_INTERACTIVE -eq "1") { Start-Process -FilePath $file -Wait -PassThru }
         else { Start-Process -FilePath $file -ArgumentList "/S" -Wait -PassThru }
    if ($p.ExitCode -ne 0) { Fail "the installer exited with code $($p.ExitCode)" }

    $exe = Join-Path $env:LOCALAPPDATA "Orbital\orbital.exe"
    if (Test-Path $exe) {
      Say "Orbital $version is installed. Start it from the Start menu, or: & `"$exe`""
    } else {
      Say "Orbital $version is installed. Start it from the Start menu."
    }
  } finally {
    Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
  }
}
