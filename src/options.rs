use clap::Parser;
use std::path::PathBuf;
use std::sync::LazyLock;

#[derive(Parser)]
pub struct Cli {
    /// The path to the file to read, accepts .gpx and .json (format: metadata result) files
    pub input_path: PathBuf,

    /// Key for google streetview static API
    #[arg(long)]
    pub api_key: String,

    /// Output location for individual frames. Default: tmp folder
    #[arg(long)]
    pub output_dir: Option<String>,

    /// Output filename for timelapse. Default: streetwarp-lapse.mp4
    #[arg(short, long)]
    pub output: Option<String>,

    /// Number of network calls to allow at once, default: 40.
    #[arg(long)]
    pub network_concurrency: Option<usize>,

    /// Number of frames to search for per mile, default: 100.
    #[arg(short, long)]
    pub frames_per_mile: Option<f64>,

    /// Maximum number of frames, default: unlimited (set to 0)
    #[arg(long)]
    pub max_frames: Option<usize>,

    /// Skip first offset_frames in creating the video, default: 0
    #[arg(long)]
    pub offset_frames: Option<usize>,

    /// Don't fetch images or create video, just show metadata and expected error.
    #[arg(short, long)]
    pub dry_run: bool,

    /// Print metadata before creating result video (implied if --dry-run)
    #[arg(long)]
    pub print_metadata: bool,

    /// If this is set, then input_path is interpreted as a metadata result and the program proceeds directly to video creation.
    #[arg(long)]
    pub use_metadata: bool,

    /// Linearly interpolate given number of points between each point in the source file, default: use frames_per_mile.
    #[arg(long)]
    pub interp: Option<usize>,

    /// Use motion interpolation to smooth output video. Available: skip, fast, good. Default: good
    #[arg(long)]
    pub minterp: Option<String>,

    /// Output in JSON format. Default: off.
    #[arg(long)]
    pub json: bool,

    /// Whether to print out progress messages (in JSON) to stdout. Default: off.
    #[arg(long)]
    pub progress: bool,

    /// The path to the image optimization executable file.
    #[arg(long)]
    pub optimizer: Option<PathBuf>,

    /// Additional argument to pass to optimization executable (after output folder)
    #[arg(long)]
    pub optimizer_arg: Option<String>,
}

pub static CLI_OPTIONS: LazyLock<Cli> = LazyLock::new(Cli::parse);
