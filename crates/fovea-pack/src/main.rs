use std::net::IpAddr;
use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use fovea_pack::{serve_sources, ImageFormat, ServeOptions};

#[derive(Debug, Parser)]
#[command(
    author,
    version,
    about = "Serve WSI slides and cell protobufs to Fovea"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Serve a WSI and optional protobuf cells directly.
    Serve(ServeArgs),
}

#[derive(Debug, Parser)]
struct ServeArgs {
    /// Input whole-slide image, for example .svs or .ndpi.
    #[arg(long)]
    wsi: PathBuf,

    /// Optional protobuf file containing histotyper.SlideSegmentationData.
    #[arg(long)]
    cells_protobuf: Option<PathBuf>,

    /// Host interface to bind.
    #[arg(long, default_value = "127.0.0.1")]
    host: IpAddr,

    /// HTTP port to bind.
    #[arg(long, default_value_t = 7878)]
    port: u16,

    /// Served slide tile edge length in pixels.
    #[arg(long, default_value_t = 512)]
    tile_size: u32,

    /// Encoded slide tile format.
    #[arg(long, value_enum, default_value_t = CliImageFormat::Webp)]
    image_format: CliImageFormat,

    /// Spatial cell chunk edge length in level-0 slide pixels.
    #[arg(long, default_value_t = 4096)]
    chunk_size: u32,

    /// Maximum polygon vertices retained per cell. Use 0 for no cap.
    #[arg(long, default_value_t = 256)]
    max_vertices_per_cell: u16,

    /// Build and serve an in-memory density heatmap from the cells.
    #[arg(long)]
    heatmap: bool,

    /// Level-0 slide pixels represented by one heatmap pixel.
    #[arg(long, default_value_t = 128)]
    heatmap_bin_size: u32,

    /// Served heatmap tile edge length in heatmap pixels.
    #[arg(long, default_value_t = 256)]
    heatmap_tile_size: u32,

    /// Maximum RAM used for encoded slide tile cache.
    #[arg(long, default_value_t = 1024)]
    tile_cache_mb: usize,
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
        Command::Serve(args) => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            runtime.block_on(serve_sources(ServeOptions {
                wsi_path: args.wsi,
                cells_protobuf_path: args.cells_protobuf,
                host: args.host,
                port: args.port,
                tile_size: args.tile_size,
                image_format: args.image_format.into(),
                chunk_size: args.chunk_size,
                max_vertices_per_cell: args.max_vertices_per_cell,
                heatmap: args.heatmap,
                heatmap_bin_size: args.heatmap_bin_size,
                heatmap_tile_size: args.heatmap_tile_size,
                tile_cache_mb: args.tile_cache_mb,
            }))
        }
    }
}
