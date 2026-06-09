use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use fovea_pack::{
    pack_cells_protobuf, pack_heatmap_from_cell_overlay, pack_slide, CellOverlayPackOptions,
    HeatmapOverlayPackOptions, ImageFormat, PackOptions,
};

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
    /// Convert a histotyper SlideSegmentationData protobuf into Fovea overlay chunks.
    CellsProtobuf(CellsProtobufArgs),
    /// Convert a Fovea cell overlay bundle into tiled density heatmap tiles.
    HeatmapOverlay(HeatmapOverlayArgs),
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

#[derive(Debug, Parser)]
struct CellsProtobufArgs {
    /// Input protobuf file containing histotyper.SlideSegmentationData.
    #[arg(long)]
    proto: PathBuf,

    /// Output overlay bundle directory.
    #[arg(long)]
    out: PathBuf,

    /// Overlay id written into the manifest.
    #[arg(long, default_value = "cells")]
    id: String,

    /// Spatial chunk edge length in level-0 slide pixels.
    #[arg(long, default_value_t = 4096)]
    chunk_size: u32,

    /// Maximum polygon vertices retained per cell. Use 0 for no cap.
    #[arg(long, default_value_t = 256)]
    max_vertices_per_cell: u16,

    /// Remove an existing output bundle before writing.
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Parser)]
struct HeatmapOverlayArgs {
    /// Input .overlay bundle created by the cells-protobuf command.
    #[arg(long)]
    overlay: PathBuf,

    /// Output heatmap bundle directory.
    #[arg(long)]
    out: PathBuf,

    /// Heatmap id written into the manifest.
    #[arg(long, default_value = "cell_density")]
    id: String,

    /// Level-0 slide pixels represented by one heatmap pixel.
    #[arg(long, default_value_t = 128)]
    bin_size: u32,

    /// Output heatmap tile edge length in heatmap pixels.
    #[arg(long, default_value_t = 256)]
    tile_size: u32,

    /// Remove an existing output bundle before writing.
    #[arg(long)]
    force: bool,
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
        Command::CellsProtobuf(args) => pack_cells_protobuf(CellOverlayPackOptions {
            proto_path: args.proto,
            out_dir: args.out,
            id: args.id,
            chunk_size: args.chunk_size,
            max_vertices_per_cell: args.max_vertices_per_cell,
            force: args.force,
        }),
        Command::HeatmapOverlay(args) => {
            pack_heatmap_from_cell_overlay(HeatmapOverlayPackOptions {
                overlay_dir: args.overlay,
                out_dir: args.out,
                id: args.id,
                bin_size: args.bin_size,
                tile_size: args.tile_size,
                force: args.force,
            })
        }
    }
}
