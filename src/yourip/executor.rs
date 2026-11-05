use std::env;
use std::io;
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::tools::Tool;
use crate::yourip::ffmpeg;
use crate::yourip::rip;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Media {
    Audio,
    Video,
}

#[derive(Clone)]
pub struct Executor {
    yt_dlp_path: String,
    ffmpeg_path: String,
    js_runtime: Option<String>,
    preferred_audio_format: String,
    preferred_video_format: String,
    output_folder: String,
    audio_folder: String,
    video_folder: String,
    workspace_folder: String,
}

impl Executor {
    pub fn new(
        yt_dlp_path: String,
        ffmpeg_path: String,
        js_runtime: Option<String>,
        preferred_audio_format: String,
        preferred_video_format: String,
        output_folder: String,
        audio_folder: String,
        video_folder: String,
        workspace_folder: String,
    ) -> Self {
        Self {
            yt_dlp_path,
            ffmpeg_path,
            js_runtime,
            preferred_audio_format,
            preferred_video_format,
            output_folder,
            audio_folder,
            video_folder,
            workspace_folder,
        }
    }

    pub fn from_env() -> Self {
        let downloads = dirs::download_dir()
            .unwrap_or_else(|| PathBuf::from("data"))
            .join("Rippy");
        Self::new(
            env::var("YT_DL_PATH").unwrap_or_else(|_| Tool::YtDlp.command()),
            env::var("FFMPEG_PATH").unwrap_or_else(|_| Tool::Ffmpeg.command()),
            // yt-dlp needs a JavaScript runtime for YouTube; deno is the one it enables by default.
            env::var("JS_RUNTIME")
                .ok()
                .or_else(|| Tool::Deno.managed_path().map(|deno| format!("deno:{deno}"))),
            env::var("PREFERRED_AUDIO_FORMAT").unwrap_or("mp3".to_string()),
            env::var("PREFERRED_VIDEO_FORMAT").unwrap_or("mp4".to_string()),
            env::var("OUTPUT_FOLDER").unwrap_or(downloads.to_string_lossy().to_string()),
            env::var("AUDIO_FOLDER").unwrap_or("audio".to_string()),
            env::var("VIDEO_FOLDER").unwrap_or("video".to_string()),
            env::var("WORKSPACE_FOLDER").unwrap_or("workspace".to_string()),
        )
    }

    fn new_workspace(&self) -> io::Result<String> {
        let id = Uuid::new_v4().to_string().replace("-", "");
        let path = Path::new(&self.output_folder)
            .join(&self.workspace_folder)
            .join(id);
        std::fs::create_dir_all(&path)?;
        Ok(path.to_string_lossy().to_string())
    }

    fn infer_file_name(&self, workspace: &str) -> io::Result<String> {
        let files = std::fs::read_dir(workspace)?;
        let mut file_names = files
            .filter_map(|file| file.ok())
            .map(|file| file.path().to_string_lossy().to_string())
            .collect::<Vec<String>>();
        file_names.sort();
        file_names
            .first()
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "yt-dlp saved no file"))
    }

    fn move_to(&self, file_name: &str, folder: &str) -> io::Result<PathBuf> {
        let file_base_name = file_name
            .rsplit_once(std::path::MAIN_SEPARATOR)
            .map(|(_, a)| a)
            .unwrap_or(file_name);
        let folder = Path::new(&self.output_folder).join(folder);
        std::fs::create_dir_all(&folder)?;
        let final_path = folder.join(file_base_name);
        std::fs::rename(file_name, &final_path)?;
        Ok(final_path)
    }

    pub fn download(&self, media: Media, url: String) -> io::Result<PathBuf> {
        match media {
            Media::Audio => self.download_audio(url),
            Media::Video => self.download_video(url),
        }
    }

    pub fn download_audio(&self, url: String) -> io::Result<PathBuf> {
        let workspace = self.new_workspace()?;
        let result = (|| {
            rip::download_audio(
                &url,
                &workspace,
                &self.yt_dlp_path,
                &self.ffmpeg_path,
                self.js_runtime.as_deref(),
            )?;
            let file_name = self.infer_file_name(&workspace)?;
            let converted_file_name = ffmpeg::convert_to_preferred_format(
                &file_name,
                &self.ffmpeg_path,
                &self.preferred_audio_format,
            )?;
            if converted_file_name != file_name {
                let _ = std::fs::remove_file(&file_name);
            }
            self.move_to(&converted_file_name, &self.audio_folder)
        })();
        remove_workspace(&workspace);
        result
    }

    pub fn download_video(&self, url: String) -> io::Result<PathBuf> {
        let workspace = self.new_workspace()?;
        let result = (|| {
            rip::download_video(
                &url,
                &workspace,
                &self.yt_dlp_path,
                &self.ffmpeg_path,
                self.js_runtime.as_deref(),
            )?;
            let file_name = self.infer_file_name(&workspace)?;
            self.move_to(&file_name, &self.video_folder)
        })();
        remove_workspace(&workspace);
        result
    }
}

fn remove_workspace(workspace: &str) {
    if let Err(err) = std::fs::remove_dir_all(workspace) {
        eprintln!("Couldn't remove workspace {workspace}: {err}");
    }
}
