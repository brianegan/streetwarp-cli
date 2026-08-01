use std::path::Path;
use std::process::Stdio;
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;

use crate::options::CLI_OPTIONS;
use crate::progress::progress;

/// The output video's dimensions, fixed by the encode in [`create_timelapse`].
pub const VIDEO_WIDTH: u32 = 640;
pub const VIDEO_HEIGHT: u32 = 480;

/// How the final pass smooths motion between Street View frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    /// Leave the 24fps sequence as it is.
    Skip,
    /// Interpolate to 48fps and average pairs back down to 24.
    Blend,
    /// Motion-compensated interpolation up to 72fps.
    Minterp,
}

/// Where the minimap goes and how big it is, in output video pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Overlay {
    pub x: u32,
    pub y: u32,
    pub size_px: u32,
}

impl Motion {
    /// The filter chain that turns the source video into `[base]`.
    fn chain(self) -> &'static str {
        match self {
            Motion::Skip => "null",
            Motion::Blend => "minterpolate=fps=48,tblend=all_mode=average,framestep=2",
            Motion::Minterp => "minterpolate='mi_mode=mci:mc_mode=aobmc:vsbmc=1:fps=72'",
        }
    }

    /// Rendered frames per source frame, which is what makes the progress
    /// percentage mean the same thing whichever mode is running.
    fn progress_scale(self) -> f64 {
        match self {
            Motion::Skip | Motion::Blend => 100.0,
            Motion::Minterp => 33.3,
        }
    }
}

/// Build the filter graph for the final encode.
///
/// The overlay is appended after the motion stage on purpose. Interpolation
/// then only ever sees Street View imagery, and the map composites on top of the
/// result, so map labels stay crisp instead of being smeared by motion
/// estimation that has no idea they are graphics.
pub fn final_filter_graph(motion: Motion, overlay: Option<Overlay>) -> String {
    match overlay {
        None => format!("[0:v]{}[out]", motion.chain()),
        Some(Overlay { x, y, size_px }) => format!(
            "[0:v]{}[base];[1:v]scale={size_px}:{size_px}[map];[base][map]overlay={x}:{y}[out]",
            motion.chain()
        ),
    }
}

type GetProgress = dyn Fn(usize) -> f64;
pub async fn ffmpeg<P: AsRef<Path>>(working_dir: P, get_progress: &GetProgress, args: &[&str]) {
    let mut command = Command::new("ffmpeg");
    let command = command
        .args(args)
        .current_dir(working_dir)
        .stdout(Stdio::piped());
    // Print arguments list to stderr
    eprintln!("ffmpeg {}", args.join(" "));
    let mut child = command.spawn().expect("ffmpeg spawn failure");
    let stdout = child.stdout.take().expect("ffmpeg stdout failure");
    let mut reader = tokio::io::BufReader::new(stdout).lines();
    // Ensure the child process is spawned in the runtime so it can
    // make progress on its own while we await for any output.
    let thread = tokio::spawn(async move {
        child
            .wait()
            .await
            .expect("child process encountered an error");
    });

    while let Some(line) = reader.next_line().await.expect("ffmpeg readline failure") {
        if line.contains("frame=") {
            let frame =
                str::parse::<usize>(&line["frame=".len()..]).expect("Could not parse frame");
            progress(&format!("{:.1}% rendered", get_progress(frame)));
        }
    }
    thread.await.expect("Failed to join ffmpeg thread");
}

pub async fn create_timelapse<P: AsRef<Path>>(image_dir: P, num_images: usize, out_filename: &str) {
    // ffmpeg -framerate 30 -pattern_type glob -i "folder-with-photos/*.JPG" -s:v 1440x1080 -c:v libx264 -crf 25 -pix_fmt yuv420p my-timelapse.mp4
    let pattern = if CLI_OPTIONS.optimizer.is_some() {
        "%d.opt.jpg"
    } else {
        "%d.jpg"
    };
    ffmpeg(
        image_dir,
        &(move |frame| 100.0 * (frame as f64) / (num_images as f64)),
        &[
            "-framerate",
            "24",
            "-pattern_type",
            "sequence",
            "-i",
            pattern,
            "-s:v",
            "640x480",
            "-c:v",
            "libx264",
            "-crf",
            "22",
            "-pix_fmt",
            "yuv420p",
            "-preset",
            "faster",
            "-movflags",
            "faststart",
            "-progress",
            "pipe:1",
            "-y",
            out_filename,
        ],
    )
    .await;
}

/// Run the final encode: motion smoothing, then the minimap composite.
///
/// One pass does both, so the video is encoded once rather than being decoded
/// and re-encoded to lay the map on afterwards.
pub async fn finish_timelapse<P: AsRef<Path>>(
    image_dir: P,
    num_images: usize,
    motion: Motion,
    overlay: Option<Overlay>,
    original_filename: &str,
    out_filename: &str,
) {
    let graph = final_filter_graph(motion, overlay);
    let mut args = vec!["-i".to_string(), original_filename.to_string()];
    if overlay.is_some() {
        // Read the minimaps at the source frame rate so map frame N lines up
        // with Street View frame N. Interpolation raises the output rate past
        // this, and overlay holds each map frame across the gap.
        args.extend(
            ["-framerate", "24", "-pattern_type", "sequence", "-start_number", "0", "-i", "%d.map.png"]
                .map(String::from),
        );
    }
    args.extend(
        [
            "-filter_complex",
            &graph,
            "-map",
            "[out]",
            "-c:v",
            "libx264",
            "-crf",
            "22",
            "-pix_fmt",
            "yuv420p",
            "-preset",
            "faster",
            "-movflags",
            "faststart",
            "-progress",
            "pipe:1",
            "-y",
            out_filename,
        ]
        .map(String::from),
    );

    let scale = motion.progress_scale();
    let borrowed = args.iter().map(String::as_str).collect::<Vec<_>>();
    ffmpeg(
        image_dir,
        &(move |frame| scale * (frame as f64) / (num_images as f64)),
        &borrowed,
    )
    .await;
}
