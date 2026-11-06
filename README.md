# Rippy

Search YouTube and save audio or video, on macOS and Windows. Built with [GPUI Kit](https://gpui-kit.com).

Rippy installs its own yt-dlp, deno and ffmpeg on first launch and keeps them up to date, so
nothing else needs installing to use it. Downloads go to `Downloads/Rippy/audio` and
`Downloads/Rippy/video`.

## Download for Windows

Get `Rippy-<version>-windows-x64.zip` from the
[latest release](https://github.com/GoatedChopin/rippy/releases/latest), extract it, and open
`Rippy.exe`. There's nothing to install or build. The build isn't code-signed yet, so Windows may
show "Windows protected your PC" the first time; choose **More info**, then **Run anyway**.

The rest of this page is for building Rippy from source.

## Build on macOS

Needs Rust 1.95 or newer and Xcode's command line tools.

```sh
cargo run                    # development
./script/bundle-mac.sh       # dist/Rippy.app with its icon
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

To publish a Windows download, bump `version` in `Cargo.toml`, push, then run this on the PC
(needs the [GitHub CLI](https://cli.github.com), signed in with `gh auth login`):

```powershell
powershell -ExecutionPolicy Bypass -File script\release-windows.ps1 -DryRun   # build and zip only
powershell -ExecutionPolicy Bypass -File script\release-windows.ps1           # build, zip, publish
```

Test the dry run's zip by extracting it somewhere new and opening `Rippy.exe` before publishing.

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
