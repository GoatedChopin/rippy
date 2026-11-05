use std::io;
use std::process::Command;

enum Format {
    Mp3,
    M4a,
    Ogg,
}

impl Format {
    fn to_string(&self) -> &str {
        match self {
            Format::Mp3 => "mp3",
            Format::M4a => "m4a",
            Format::Ogg => "ogg",
        }
    }

    fn from_string(s: &str) -> Option<Format> {
        match s {
            "mp3" => Some(Format::Mp3),
            "m4a" => Some(Format::M4a),
            "ogg" => Some(Format::Ogg),
            _ => None,
        }
    }

    fn to_ffmpeg_codec(&self) -> &str {
        match self {
            Format::Mp3 => "libmp3lame",
            Format::M4a => "aac",
            Format::Ogg => "libopus",
        }
    }
}

pub fn convert_to_preferred_format(
    input_file: &str,
    ffmpeg_path: &str,
    preferred_format: &str,
) -> io::Result<String> {
    let format = Format::from_string(preferred_format).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("unsupported audio format '{preferred_format}'"),
        )
    })?;
    let base = input_file
        .rsplit_once('.')
        .map(|(b, _)| b)
        .unwrap_or(input_file);
    let output_file = format!("{}.{}", base, format.to_string());
    let mut command = Command::new(ffmpeg_path);
    super::hide_console(&mut command);
    command.arg("-i");
    command.arg(input_file);
    command.arg("-c:a");
    command.arg(format.to_ffmpeg_codec());
    command.arg("-q:a");
    command.arg("2");
    command.arg(&output_file);

    let output = command.output()?;
    if output.status.success() {
        println!("Ok.");
        Ok(output_file)
    } else {
        eprintln!("Failed.");
        Err(io::Error::other("ffmpeg couldn't convert the audio"))
    }
}
