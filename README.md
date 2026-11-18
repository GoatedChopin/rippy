# Rippy

Search YouTube and save audio or video, on Windows, macOS and Linux. Built with
[GPUI Kit](https://gpui-kit.com).

Rippy installs its own yt-dlp, deno and ffmpeg the first time you start it and keeps them up to
date, so nothing else needs installing to use it. Downloads go to `Downloads/Rippy/audio` and
`Downloads/Rippy/video` unless you choose other folders under **Save downloads to** in Settings
(the gear).

## Download

Get the zip for your computer from the
[latest release](https://github.com/GoatedChopin/rippy/releases/latest). There's nothing to
install or build.

- **Windows 10 or later:** `Rippy-<version>-windows-x64.zip`. Extract it and open `Rippy.exe`. The
  build isn't code-signed yet, so Windows may show "Windows protected your PC" the first time;
  choose **More info**, then **Run anyway**.
- **macOS 11 or later** (Apple silicon or Intel): `Rippy-<version>-macos.zip`. Double-click it,
  drag Rippy to Applications, and open it from there. It isn't notarized yet, so the first time
  macOS says it can't verify Rippy: choose **Done**, then go to **System Settings > Privacy &
  Security**, choose **Open Anyway** next to Rippy, and confirm.
- **Linux** (x64, with glibc 2.35 or newer: Ubuntu 22.04, Debian 12, Fedora 36 and later):
  `Rippy-<version>-linux-x64.zip`. Extract it and run `rippy`.

## Updates

From 0.3.0 on, Rippy updates itself. Shortly after it starts, and then once a day, it checks the
latest release. When there's a newer version, it downloads it and puts it in place of the copy
you have, and **Restart to update** appears next to the gear. The new version also starts on its
own the next time you open Rippy. **Check for updates** in Settings checks right away.

On a Mac, keep Rippy in Applications (or another folder you can write to), not in Downloads,
or it can't replace itself. Your folder choices, API key and downloaded tools carry over.

Each release carries `rippy-update.json`, which lists every download with its size and SHA-256
and is signed with Rippy's update key. Rippy installs nothing that key didn't sign, and on a Mac
the new app's code signature must also check out as Rippy's.

Copies from before 0.3.0 update by hand once: close Rippy, download the new zip and replace your
copy with the one inside it.

The rest of this page is for building Rippy from source.

## Build on macOS

Needs Rust 1.95 or newer and Xcode's command line tools.

```sh
cargo run                    # development
./script/bundle-mac.sh       # dist/Rippy.app with its icon, for this Mac
```

## Build on Windows

Needs Windows 10 or later, Rust 1.95 or newer with the MSVC toolchain (`rustup show
active-toolchain` should say `msvc`; `rustup update stable` if it's older), and Visual Studio 2022
Build Tools with **Desktop development with C++** (MSVC and a Windows SDK). Build from a
**Developer PowerShell for VS 2022**, which puts the CMake that comes with Build Tools on `PATH`.

```powershell
cargo build --release        # target\release\rippy.exe, icon included
```

The first launch asks Windows Firewall to allow Rippy's API on your network.

## Build on Linux

Needs Rust 1.95 or newer, a C compiler, `pkg-config`, `perl` and the development packages for
fontconfig, freetype, Wayland, xcb and xkbcommon. On Ubuntu or Debian:

```sh
sudo apt install build-essential pkg-config perl libfontconfig1-dev libfreetype6-dev \
  libwayland-dev libx11-xcb-dev libxcb1-dev libxkbcommon-dev libxkbcommon-x11-dev
cargo build --release        # target/release/rippy
```

Or build in Docker, as releases are: `./script/release-linux.sh --dry-run`.

## Releasing

Bump `version` in `Cargo.toml` and push. Then build each download (each script also has
`--dry-run` / `-DryRun` to build and zip without uploading):

1. **Mac:** `./script/release-mac.sh` builds the universal app, signed ad hoc, and uploads
   `Rippy-<version>-macos.zip`.
2. **Linux, on the Mac:** `./script/release-linux.sh` builds in Docker and uploads
   `Rippy-<version>-linux-x64.zip`.
3. **Windows, on the PC:**
   `powershell -ExecutionPolicy Bypass -File script\release-windows.ps1` uploads
   `Rippy-<version>-windows-x64.zip`.
4. **Publish, on the Mac:** `./script/publish-release.sh` downloads and checks every zip, signs
   `rippy-update.json` with the update key in the login Keychain, uploads it, and makes the
   release the latest. Until then the release is a prerelease, which copies of Rippy don't see.

The uploads need the [GitHub CLI](https://cli.github.com), signed in with `gh auth login`. Test
each dry run's zip by extracting it somewhere new and starting Rippy before uploading.

To try an update without publishing anything, build two versions with a throwaway key,
`RIPPY_UPDATE_PUBLIC_KEY=<its public key> cargo build`, sign a feed for the newer one with
`cargo run --example update_feed -- sign …` (see its usage), serve it locally, and start the older
one with `RIPPY_UPDATE_FEED=http://127.0.0.1:<port>/rippy-update.json`. Debug builds only check
for updates when `RIPPY_UPDATE_FEED` is set.

## Linking other apps

Rippy serves a small API on port 7479 of your local network. The address and API key are in
Settings. Send the key as `Authorization: Bearer <key>`.

| Request | Does |
| --- | --- |
| `GET /api/status` | Rippy's version and the installed tools |
| `GET /api/search?q=…` | Search results |
| `POST /api/downloads` with `{"url": "…", "media": "audio"}` or `"video"` | Starts a download, returns its `id` |
| `GET /api/downloads/{id}` | `downloading`, `done` or `failed` |
| `GET /api/downloads/{id}/file` | The downloaded file |
| `GET /api/audio/search?q=…` | Search results as `{"results": [{id, title, artist, duration, artwork}]}` |
| `GET /api/audio/download?id=…` | A result's audio: `202` with `Retry-After` until it's ready, then the file |

Set `RIPPY_API_ADDR` (for example `127.0.0.1:7479`) to change where it listens.

## Icons

`assets/icon/rippy.svg` (macOS) and `rippy-windows.svg` are the sources. After editing them, run
`./script/make-icons.sh` on a Mac to regenerate `rippy.icns` and `rippy.ico`.
