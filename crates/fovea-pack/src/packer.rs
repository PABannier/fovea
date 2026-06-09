use std::{
    fs,
    io::BufWriter,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Instant,
};

use anyhow::{anyhow, Context, Result};
use image::{DynamicImage, ImageEncoder, RgbaImage};
use rayon::prelude::*;

use crate::{
    manifest::{AssociatedImageManifest, ImageFormat, LevelManifest, Manifest, Size, TileManifest},
    reader::{OpenSlideReader, SlideReader},
};

#[derive(Clone, Debug)]
pub struct PackOptions {
    pub wsi_path: PathBuf,
    pub out_dir: PathBuf,
    pub tile_size: u32,
    pub image_format: ImageFormat,
    pub skip_background_tiles: bool,
    pub background_threshold: u8,
    pub force: bool,
    pub jobs: usize,
}

#[derive(Clone, Debug)]
struct TileJob {
    level: u32,
    tile_x: u32,
    tile_y: u32,
    width: u32,
    height: u32,
    level0_x: i64,
    level0_y: i64,
    path: PathBuf,
    relative_path: String,
}

#[derive(Clone, Debug)]
struct PackedTile {
    manifest: TileManifest,
}

pub fn pack_slide(options: PackOptions) -> Result<()> {
    validate_options(&options)?;
    let start = Instant::now();
    let reader = Arc::new(OpenSlideReader::open(&options.wsi_path)?);
    let output_dir = prepare_output_dir(&options)?;

    eprintln!("fovea-pack: opened {}", options.wsi_path.display());
    eprintln!(
        "fovea-pack: writing {} using {} workers",
        output_dir.display(),
        options.jobs
    );

    let manifest = pack_with_reader(reader, &options, &output_dir)?;
    write_manifest(&output_dir.join("manifest.json"), &manifest)?;
    finalize_output_dir(&options, &output_dir)?;

    eprintln!(
        "fovea-pack: done in {:.2}s, wrote {} tiles across {} levels",
        start.elapsed().as_secs_f64(),
        manifest.tiles.iter().filter(|tile| !tile.skipped).count(),
        manifest.levels.len()
    );

    Ok(())
}

fn pack_with_reader(
    reader: Arc<dyn SlideReader>,
    options: &PackOptions,
    output_dir: &Path,
) -> Result<Manifest> {
    let dimensions = reader.dimensions()?;
    let properties = reader.properties();
    let metadata = properties.metadata();
    let background_rgb = properties.background_rgb();
    let levels = collect_levels(reader.as_ref(), options.tile_size)?;
    let consistency_error = coordinate_consistency_max_error(&levels, dimensions);
    let tile_jobs = build_tile_jobs(reader.as_ref(), &levels, options, output_dir)?;
    let associated_images = write_associated_images(reader.as_ref(), options, output_dir)?;

    eprintln!(
        "fovea-pack: slide {}x{}, {} levels, {} candidate tiles",
        dimensions.width,
        dimensions.height,
        levels.len(),
        tile_jobs.len()
    );

    if consistency_error > 2.0 {
        eprintln!(
            "fovea-pack: warning: level coordinate consistency max error is {:.3}px",
            consistency_error
        );
    }

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(options.jobs)
        .build()?;
    let complete = AtomicUsize::new(0);
    let total = tile_jobs.len().max(1);

    let mut tiles = pool.install(|| {
        tile_jobs
            .par_iter()
            .map(|job| {
                let tile = write_tile(
                    reader.as_ref(),
                    job,
                    options,
                    background_rgb,
                    &complete,
                    total,
                )?;
                Ok::<PackedTile, anyhow::Error>(tile)
            })
            .collect::<Result<Vec<_>>>()
    })?;

    tiles.sort_by_key(|tile| {
        (
            tile.manifest.level,
            tile.manifest.y,
            tile.manifest.x,
            tile.manifest.path.clone(),
        )
    });

    Ok(Manifest {
        schema: "fovea.slide".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        tile_size: options.tile_size,
        image_format: options.image_format,
        width: dimensions.width,
        height: dimensions.height,
        levels,
        tiles: tiles.into_iter().map(|tile| tile.manifest).collect(),
        associated_images,
        metadata,
        coordinate_consistency_max_error_px: consistency_error,
    })
}

fn validate_options(options: &PackOptions) -> Result<()> {
    if !options.wsi_path.exists() {
        return Err(anyhow!(
            "input WSI does not exist: {}",
            options.wsi_path.display()
        ));
    }

    if options.tile_size == 0 {
        return Err(anyhow!("--tile-size must be greater than zero"));
    }

    Ok(())
}

fn prepare_output_dir(options: &PackOptions) -> Result<PathBuf> {
    if options.out_dir.exists() {
        if options.force {
            fs::remove_dir_all(&options.out_dir)
                .with_context(|| format!("failed to remove {}", options.out_dir.display()))?;
        } else {
            return Err(anyhow!(
                "output directory already exists: {} (use --force to replace it)",
                options.out_dir.display()
            ));
        }
    }

    let partial_dir = partial_output_dir(&options.out_dir);

    if partial_dir.exists() {
        fs::remove_dir_all(&partial_dir)
            .with_context(|| format!("failed to remove stale {}", partial_dir.display()))?;
    }

    fs::create_dir_all(&partial_dir)
        .with_context(|| format!("failed to create {}", partial_dir.display()))?;

    Ok(partial_dir)
}

fn finalize_output_dir(options: &PackOptions, partial_dir: &Path) -> Result<()> {
    fs::rename(partial_dir, &options.out_dir).with_context(|| {
        format!(
            "failed to move {} to {}",
            partial_dir.display(),
            options.out_dir.display()
        )
    })
}

fn partial_output_dir(out_dir: &Path) -> PathBuf {
    let name = out_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("bundle");
    out_dir.with_file_name(format!("{name}.partial"))
}

fn collect_levels(reader: &dyn SlideReader, tile_size: u32) -> Result<Vec<LevelManifest>> {
    let level_count = reader.level_count()?;
    let mut levels = Vec::with_capacity(level_count);

    for level in 0..level_count {
        let size = reader.level_dimensions(level)?;
        let (tile_cols, tile_rows) = size.tile_count(tile_size);

        levels.push(LevelManifest {
            index: level as u32,
            width: size.width,
            height: size.height,
            downsample: reader.level_downsample(level)?,
            tile_cols,
            tile_rows,
            tile_count: tile_cols * tile_rows,
        });
    }

    Ok(levels)
}

fn build_tile_jobs(
    _reader: &dyn SlideReader,
    levels: &[LevelManifest],
    options: &PackOptions,
    output_dir: &Path,
) -> Result<Vec<TileJob>> {
    let mut jobs = Vec::new();
    let extension = options.image_format.extension();

    for level in levels {
        let level_dir = output_dir
            .join("images")
            .join(format!("level_{}", level.index));
        fs::create_dir_all(&level_dir)
            .with_context(|| format!("failed to create {}", level_dir.display()))?;

        for tile_y in 0..level.tile_rows {
            for tile_x in 0..level.tile_cols {
                let x = tile_x * options.tile_size;
                let y = tile_y * options.tile_size;
                let width = options.tile_size.min(level.width - x);
                let height = options.tile_size.min(level.height - y);
                let file_name = format!("{tile_x}_{tile_y}.{extension}");
                let relative_path = format!("images/level_{}/{}", level.index, file_name);

                jobs.push(TileJob {
                    level: level.index,
                    tile_x,
                    tile_y,
                    width,
                    height,
                    level0_x: (f64::from(x) * level.downsample).round() as i64,
                    level0_y: (f64::from(y) * level.downsample).round() as i64,
                    path: level_dir.join(file_name),
                    relative_path,
                });
            }
        }
    }

    Ok(jobs)
}

fn write_tile(
    reader: &dyn SlideReader,
    job: &TileJob,
    options: &PackOptions,
    background_rgb: [u8; 3],
    complete: &AtomicUsize,
    total: usize,
) -> Result<PackedTile> {
    let image = reader.read_region_rgba(
        job.level as usize,
        job.level0_x,
        job.level0_y,
        job.width,
        job.height,
    )?;
    let skipped = options.skip_background_tiles
        && is_background_tile(&image, background_rgb, options.background_threshold);

    let byte_size = if skipped {
        0
    } else {
        encode_image(&image, options.image_format, &job.path)?;
        fs::metadata(&job.path)?.len()
    };

    let done = complete.fetch_add(1, Ordering::Relaxed) + 1;

    if done == total || done % 100 == 0 {
        eprintln!("fovea-pack: tiles {done}/{total}");
    }

    Ok(PackedTile {
        manifest: TileManifest {
            level: job.level,
            x: job.tile_x,
            y: job.tile_y,
            width: job.width,
            height: job.height,
            path: job.relative_path.clone(),
            byte_size,
            skipped,
        },
    })
}

fn encode_image(image: &RgbaImage, format: ImageFormat, path: &Path) -> Result<()> {
    let file =
        fs::File::create(path).with_context(|| format!("failed to create {}", path.display()))?;
    let writer = BufWriter::new(file);

    match format {
        ImageFormat::Jpeg => {
            let rgb = DynamicImage::ImageRgba8(image.clone()).to_rgb8();
            let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(writer, 88);
            encoder.write_image(
                rgb.as_raw(),
                rgb.width(),
                rgb.height(),
                image::ExtendedColorType::Rgb8,
            )?;
        }
        ImageFormat::Webp | ImageFormat::Png => {
            DynamicImage::ImageRgba8(image.clone()).write_to(
                &mut std::io::BufWriter::new(writer.into_inner()?),
                format.image_crate_format(),
            )?;
        }
    }

    Ok(())
}

fn write_associated_images(
    reader: &dyn SlideReader,
    options: &PackOptions,
    output_dir: &Path,
) -> Result<Vec<AssociatedImageManifest>> {
    let image_names = reader.associated_image_names().unwrap_or_default();
    let thumbnails_dir = output_dir.join("thumbnails");
    fs::create_dir_all(&thumbnails_dir)
        .with_context(|| format!("failed to create {}", thumbnails_dir.display()))?;
    let mut associated = Vec::new();

    for name in image_names {
        let Ok((size, image)) = reader.read_associated_image_rgba(&name) else {
            continue;
        };

        let safe_name = sanitize_path_component(&name);
        let path = thumbnails_dir.join(format!(
            "{}.{}",
            safe_name,
            options.image_format.extension()
        ));
        let relative_path = format!(
            "thumbnails/{}.{}",
            safe_name,
            options.image_format.extension()
        );
        encode_image(&image, options.image_format, &path)?;
        associated.push(AssociatedImageManifest {
            name,
            width: size.width,
            height: size.height,
            path: relative_path,
        });
    }

    Ok(associated)
}

fn write_manifest(path: &Path, manifest: &Manifest) -> Result<()> {
    let file =
        fs::File::create(path).with_context(|| format!("failed to create {}", path.display()))?;
    serde_json::to_writer_pretty(BufWriter::new(file), manifest)?;
    Ok(())
}

fn coordinate_consistency_max_error(levels: &[LevelManifest], level0: Size) -> f64 {
    levels
        .iter()
        .map(|level| {
            let width_error =
                (f64::from(level.width) * level.downsample - f64::from(level0.width)).abs();
            let height_error =
                (f64::from(level.height) * level.downsample - f64::from(level0.height)).abs();
            width_error.max(height_error)
        })
        .fold(0.0, f64::max)
}

fn is_background_tile(image: &RgbaImage, background_rgb: [u8; 3], threshold: u8) -> bool {
    let threshold = i16::from(threshold);

    image.pixels().step_by(16).all(|pixel| {
        if pixel.0[3] == 0 {
            return true;
        }

        pixel.0[..3]
            .iter()
            .zip(background_rgb)
            .all(|(actual, bg)| (i16::from(*actual) - i16::from(bg)).abs() <= threshold)
    })
}

fn sanitize_path_component(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();

    if sanitized.is_empty() {
        "image".to_string()
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use image::{Rgba, RgbaImage};

    use super::{is_background_tile, sanitize_path_component};

    #[test]
    fn detects_sampled_background_tile() {
        let image = RgbaImage::from_pixel(32, 32, Rgba([255, 255, 250, 255]));
        assert!(is_background_tile(&image, [255, 255, 255], 8));
        assert!(!is_background_tile(&image, [240, 240, 240], 8));
    }

    #[test]
    fn sanitizes_associated_image_names_for_paths() {
        assert_eq!(sanitize_path_component("macro image"), "macro_image");
        assert_eq!(sanitize_path_component("../label"), "___label");
    }
}
