# Shared by the Mac release scripts: the version, the pushed-commit check and uploads.
# Sourced from the repo root.

REPO="${RIPPY_REPO:-GoatedChopin/rippy}"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
tag="v$version"

fail() {
  echo "error: $*" >&2
  exit 1
}

# The release has to match a commit on GitHub.
require_pushed() {
  [ -z "$(git status --porcelain)" ] \
    || fail "commit or stash your changes first, so the release matches a commit on GitHub"
  git fetch --quiet origin
  git merge-base --is-ancestor HEAD origin/main || fail "push this commit to origin/main first"
}

notes() {
  cat <<NOTES
Download the file for your computer:

- **Windows:** \`Rippy-$version-windows-x64.zip\`. Extract it and open \`Rippy.exe\`.
- **Mac:** \`Rippy-$version-macos.zip\`. Double-click it and drag Rippy to Applications. The first time, macOS asks you to allow it: System Settings > Privacy & Security > Open Anyway.
- **Linux:** \`Rippy-$version-linux-x64.zip\`. Extract it and run \`rippy\`.

Nothing else needs installing: Rippy gets yt-dlp, deno and ffmpeg for itself the first time you start it. From 0.3.0 on, Rippy also updates itself.

Built from $(git rev-parse HEAD).
NOTES
}

# Adds files to this version's release. A new release starts as a prerelease, which Rippy's
# updates don't see, until publish-release.sh signs its feed and makes it the latest.
upload() {
  command -v gh >/dev/null || fail "install the GitHub CLI and run gh auth login"
  if ! gh release view "$tag" --repo "$REPO" >/dev/null 2>&1; then
    gh release create "$tag" --repo "$REPO" --prerelease --target "$(git rev-parse HEAD)" \
      --title "Rippy $version" --notes "$(notes)"
  fi
  gh release upload "$tag" "$@" --clobber --repo "$REPO"
  echo "Uploaded to https://github.com/$REPO/releases/tag/$tag"
}
