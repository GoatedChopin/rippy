# Builds Rippy for Windows and uploads it to a GitHub release that Windows users can download.
# Run on the Windows PC from the repo, signed in with `gh auth login`:
#   powershell -ExecutionPolicy Bypass -File script\release-windows.ps1           build, zip, publish
#   powershell -ExecutionPolicy Bypass -File script\release-windows.ps1 -DryRun   build and zip only
# Bump `version` in Cargo.toml and push before each new release.
param(
    [string]$Repo = "GoatedChopin/rippy",
    [switch]$DryRun
)

$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")

# Windows PowerShell turns a native command's stderr into errors, so run git and gh with it relaxed.
function Invoke-Native([scriptblock]$Command) {
    $previous = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try { & $Command } finally { $ErrorActionPreference = $previous }
}

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$tag = "v$version"
$arch = if ($env:PROCESSOR_ARCHITECTURE -eq "ARM64") { "arm64" } else { "x64" }
$zipName = "Rippy-$version-windows-$arch.zip"

# The release has to match a commit on GitHub, so refuse local changes or unpushed commits.
if (Invoke-Native { git status --porcelain }) {
    throw "Commit or stash your changes first, so the release matches a commit on GitHub."
}
Invoke-Native { git fetch --quiet origin }
$commit = (Invoke-Native { git rev-parse HEAD }).Trim()
Invoke-Native { git merge-base --is-ancestor $commit origin/main }
if ($LASTEXITCODE -ne 0) {
    throw "Push this commit to origin/main first."
}

Invoke-Native { cargo build --release }
if ($LASTEXITCODE -ne 0) { throw "cargo build failed." }

$stage = "dist\windows\Rippy"
if (Test-Path dist\windows) { Remove-Item -Recurse -Force dist\windows }
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item target\release\rippy.exe "$stage\Rippy.exe"
$zip = "dist\$zipName"
Compress-Archive -Path $stage -DestinationPath $zip -Force

$hash = (Get-FileHash -Algorithm SHA256 $zip).Hash.ToLower()
$sums = "dist\$zipName.sha256"
[System.IO.File]::WriteAllText((Join-Path (Get-Location) $sums), "$hash  $zipName`n")
Write-Host "Built $zip ($hash)"

if ($DryRun) {
    Write-Host "Dry run: nothing uploaded. Extract $zip somewhere new and launch Rippy.exe to test it."
    exit 0
}

if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
    throw "Install the GitHub CLI (winget install GitHub.cli) and run gh auth login."
}

Invoke-Native { gh release view $tag --repo $Repo *> $null }
if ($LASTEXITCODE -eq 0) {
    Write-Host "Release $tag exists; replacing its Windows $arch download."
    Invoke-Native { gh release upload $tag $zip $sums --clobber --repo $Repo }
} else {
    $notes = Join-Path $env:TEMP "rippy-release-notes.md"
    $text = @"
Download **$zipName**, extract it, and open **Rippy.exe**. Nothing else needs installing: on first launch Rippy downloads yt-dlp, deno and ffmpeg for itself.

Built from $commit.
"@
    [System.IO.File]::WriteAllText($notes, $text)
    Invoke-Native { gh release create $tag $zip $sums --repo $Repo --target $commit --title "Rippy $version" --notes-file $notes }
}
if ($LASTEXITCODE -ne 0) { throw "Uploading to GitHub failed." }
Write-Host "Published https://github.com/$Repo/releases/tag/$tag"
