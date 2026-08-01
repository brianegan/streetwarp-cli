use clap::{Parser, ValueEnum};
use std::path::PathBuf;
use std::sync::LazyLock;

/// What the minimap overlay shows, if anything.
#[derive(ValueEnum, Debug, Copy, Clone, PartialEq, Eq)]
pub enum MinimapMode {
    /// No overlay. This is the default; the minimap costs extra API calls.
    Off,
    /// One map framing the whole route, with a dot moving along it. Costs a
    /// single Static Maps request for the entire render.
    Overview,
    /// A map centred on the current position, panning as the video plays. Costs
    /// one Static Maps request per frame.
    Follow,
}

/// Which corner of the video the minimap sits in.
#[derive(ValueEnum, Debug, Copy, Clone, PartialEq, Eq)]
pub enum MinimapPosition {
    Tl,
    Tr,
    Bl,
    Br,
}

#[derive(Parser)]
pub struct Cli {
    /// The path to the file to read, accepts .gpx and .json (format: metadata result) files
    pub input_path: PathBuf,

    /// Key for google streetview static API. Reads GOOGLE_API_KEY when the
    /// flag is absent, which keeps the key out of your shell history and out of
    /// the process list.
    #[arg(long, env = "GOOGLE_API_KEY", hide_env_values = true)]
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

    /// Draw a small map over the video showing where each frame sits on the route.
    /// Available: off, overview, follow. Default: off.
    #[arg(long, value_enum, default_value_t = MinimapMode::Off)]
    pub minimap: MinimapMode,

    /// Corner the minimap sits in. Available: tl, tr, bl, br. Default: br.
    #[arg(long, value_enum, default_value_t = MinimapPosition::Br)]
    pub minimap_position: MinimapPosition,

    /// Minimap size as a percent of the video's shorter side, default: 30.
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u32).range(1..=100))]
    pub minimap_size: u32,

    /// Gap between the minimap and the edge of the video in pixels, default: 12.
    #[arg(long, default_value_t = 12)]
    pub minimap_margin: u32,

    /// Zoom level for follow-mode minimaps, default: 16.
    #[arg(long, default_value_t = 16, value_parser = clap::value_parser!(u32).range(0..=21))]
    pub minimap_zoom: u32,

    /// Don't read or write the on-disk response cache, so every image is
    /// fetched and paid for again. Default: off.
    #[arg(long)]
    pub no_cache: bool,
}

pub static CLI_OPTIONS: LazyLock<Cli> = LazyLock::new(|| {
    if cfg!(test) {
        // Under `cargo test` the process arguments belong to the test harness,
        // not to a streetwarp command line, so parsing them would abort the run
        // the first time anything reads an option. Progress reporting reads one
        // on every fetch, so that would take the fetch tests down with it.
        // Tests that care about a specific option parse their own `Cli`.
        Cli::parse_from(["streetwarp", "test.gpx", "--api-key", "test-key"])
    } else {
        Cli::parse()
    }
});
