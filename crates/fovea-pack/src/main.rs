use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use fovea_pack::{pack_slide, ImageFormat, PackOptions};

#[derive(Debug, Parser)]
#[command(
    author,
    version,
    about = "Native Fovea WSI ingestion and packing tools"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Convert an OpenSlide-readable WSI into a static Fovea slide bundle.
    Slide(SlideArgs),
}

#[derive(Debug, Parser)]
struct SlideArgs {
    /// Input whole-slide image, for example .svs or .ndpi.
    #[arg(long)]
    wsi: PathBuf,

    /// Output .fovea bundle directory.
    #[arg(long)]
    out: PathBuf,

    /// Output tile edge length in pixels.
    #[arg(long, default_value_t = 512)]
    tile_size: u32,

    /// Encoded tile format.
    #[arg(long, value_enum, default_value_t = CliImageFormat::Webp)]
    image_format: CliImageFormat,

    /// Skip tiles whose sampled pixels match the slide background.
    #[arg(long)]
    skip_background_tiles: bool,

    /// Per-channel tolerance for background tile detection.
    #[arg(long, default_value_t = 8)]
    background_threshold: u8,

    /// Remove an existing output bundle before writing.
    #[arg(long)]
    force: bool,

    /// Number of tile extraction workers. Defaults to logical CPU count.
    #[arg(long)]
    jobs: Option<usize>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CliImageFormat {
    Webp,
    Jpeg,
    Png,
}

impl From<CliImageFormat> for ImageFormat {
    fn from(value: CliImageFormat) -> Self {
        match value {
            CliImageFormat::Webp => Self::Webp,
            CliImageFormat::Jpeg => Self::Jpeg,
            CliImageFormat::Png => Self::Png,
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Slide(args) => pack_slide(PackOptions {
            wsi_path: args.wsi,
            out_dir: args.out,
            tile_size: args.tile_size,
            image_format: args.image_format.into(),
            skip_background_tiles: args.skip_background_tiles,
            background_threshold: args.background_threshold,
            force: args.force,
            jobs: args.jobs.unwrap_or_else(num_cpus::get).max(1),
        }),
    }
}
