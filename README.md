# Rippy

Search YouTube and save audio or video, on macOS and Windows. Built with [GPUI Kit](https://gpui-kit.com).

Rippy installs its own yt-dlp, deno and ffmpeg on first launch and keeps them up to date, so
nothing else needs installing to use it. Downloads go to `Downloads/Rippy/audio` and
`Downloads/Rippy/video`.

## Build on macOS

Needs Rust and Xcode's command line tools.

```sh
cargo run                    # development
./script/bundle-mac.sh       # dist/Rippy.app with its icon
```

## Build on Windows

Needs Windows 10 or later, Rust's MSVC toolchain (`rustup show active-toolchain` should say
`msvc`), Visual Studio 2022 Build Tools with **Desktop development with C++** (MSVC and a Windows
SDK), and CMake on `PATH`.

```powershell
cargo build --release        # target\release\rippy.exe, icon included
```

The first launch asks Windows Firewall to allow Rippy's API on your network.

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

Set `RIPPY_API_ADDR` (for example `127.0.0.1:7479`) to change where it listens.

## Icons

`assets/icon/rippy.svg` (macOS) and `rippy-windows.svg` are the sources. After editing them, run
`./script/make-icons.sh` on a Mac to regenerate `rippy.icns` and `rippy.ico`.
