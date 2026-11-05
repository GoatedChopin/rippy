use std::io;
use std::process::Command;

fn run_ytdlp(command: &mut Command, yt_dlp_path: &str) -> io::Result<()> {
    let output = command.output().map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("'{}' not found. Rippy installs yt-dlp on launch.", yt_dlp_path),
            )
        } else {
            e
        }
    })?;

    if output.status.success() {
        println!("Ok.");
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reason = stderr.lines().last().unwrap_or("yt-dlp failed").to_string();
        eprintln!("Failed: {reason}");
        Err(io::Error::other(reason))
    }
}

fn ytdlp_command(yt_dlp_path: &str, ffmpeg_path: &str, js_runtime: Option<&str>) -> Command {
    let mut command = Command::new(yt_dlp_path);
    super::hide_console(&mut command);
    command.arg("--no-playlist");
    command.arg("--ffmpeg-location");
    command.arg(ffmpeg_path);
    if let Some(js_runtime) = js_runtime {
        command.arg("--js-runtimes");
        command.arg(js_runtime);
    }
    command
}

pub fn download_audio(
    url: &str,
    output_folder: &str,
    yt_dlp_path: &str,
    ffmpeg_path: &str,
    js_runtime: Option<&str>,
) -> io::Result<()> {
    let mut command = ytdlp_command(yt_dlp_path, ffmpeg_path, js_runtime);
    command.arg("-x");
    command.arg("-o");
    command.arg(format!(
        "{}{}%(title)s.%(ext)s",
        output_folder,
        std::path::MAIN_SEPARATOR
    ));
    // `--` keeps a URL that starts with `-` from being read as an option.
    command.arg("--");
    command.arg(url);

    run_ytdlp(&mut command, yt_dlp_path)
}

pub fn download_video(
    url: &str,
    output_folder: &str,
    yt_dlp_path: &str,
    ffmpeg_path: &str,
    js_runtime: Option<&str>,
) -> io::Result<()> {
    let mut command = ytdlp_command(yt_dlp_path, ffmpeg_path, js_runtime);
    command.arg("-f");
    command.arg("bestvideo[ext=mp4]+bestaudio[ext=m4a]/best[ext=mp4]/best");
    command.arg("-o");
    command.arg(format!(
        "{}{}%(title)s.%(ext)s",
        output_folder,
        std::path::MAIN_SEPARATOR
    ));
    command.arg("--");
    command.arg(url);

    run_ytdlp(&mut command, yt_dlp_path)
}
