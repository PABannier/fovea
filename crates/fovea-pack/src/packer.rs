use std::io::Cursor;

use anyhow::{anyhow, Result};
use image::{DynamicImage, ImageEncoder, RgbaImage};

use crate::{
    manifest::{ImageFormat, LevelManifest, Manifest, Size, TileManifest},
    reader::SlideReader,
};

#[derive(Clone, Debug)]
pub struct SlideTileRequest {
    pub level: u32,
    pub width: u32,
    pub height: u32,
    pub level0_x: i64,
    pub level0_y: i64,
}

pub fn build_slide_manifest(
    reader: &dyn SlideReader,
    tile_size: u32,
    image_format: ImageFormat,
) -> Result<Manifest> {
    if tile_size == 0 {
        return Err(anyhow!("tile size must be greater than zero"));
    }

    let dimensions = reader.dimensions()?;
    let properties = reader.properties();
    let metadata = properties.metadata();
    let levels = collect_levels(reader, tile_size)?;
    let consistency_error = coordinate_consistency_max_error(&levels, dimensions);
    let mut tiles = Vec::new();
    let extension = image_format.extension();

    for level in &levels {
        for y in 0..level.tile_rows {
            for x in 0..level.tile_cols {
                let level_x = x * tile_size;
                let level_y = y * tile_size;
                tiles.push(TileManifest {
                    level: level.index,
                    x,
                    y,
                    width: tile_size.min(level.width - level_x),
                    height: tile_size.min(level.height - level_y),
                    path: format!("images/level_{}/{}_{}.{}", level.index, x, y, extension),
                    byte_size: 0,
                    skipped: false,
                });
            }
        }
    }

    Ok(Manifest {
        schema: "fovea.slide".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        tile_size,
        image_format,
        width: dimensions.width,
        height: dimensions.height,
        levels,
        tiles,
        associated_images: Vec::new(),
        metadata,
        coordinate_consistency_max_error_px: consistency_error,
    })
}

pub fn slide_tile_request(
    manifest: &Manifest,
    level: u32,
    tile_x: u32,
    tile_y: u32,
) -> Option<SlideTileRequest> {
    let level_manifest = manifest.levels.iter().find(|entry| entry.index == level)?;

    if tile_x >= level_manifest.tile_cols || tile_y >= level_manifest.tile_rows {
        return None;
    }

    let x = tile_x * manifest.tile_size;
    let y = tile_y * manifest.tile_size;
    Some(SlideTileRequest {
        level,
        width: manifest.tile_size.min(level_manifest.width - x),
        height: manifest.tile_size.min(level_manifest.height - y),
        level0_x: (f64::from(x) * level_manifest.downsample).round() as i64,
        level0_y: (f64::from(y) * level_manifest.downsample).round() as i64,
    })
}

pub fn encode_slide_tile(
    reader: &dyn SlideReader,
    request: &SlideTileRequest,
    image_format: ImageFormat,
) -> Result<Vec<u8>> {
    let image = reader.read_region_rgba(
        request.level as usize,
        request.level0_x,
        request.level0_y,
        request.width,
        request.height,
    )?;
    encode_image_bytes(&image, image_format)
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

pub fn encode_image_bytes(image: &RgbaImage, format: ImageFormat) -> Result<Vec<u8>> {
    let mut bytes = Cursor::new(Vec::new());

    match format {
        ImageFormat::Jpeg => {
            let rgb = DynamicImage::ImageRgba8(image.clone()).to_rgb8();
            let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 88);
            encoder.write_image(
                rgb.as_raw(),
                rgb.width(),
                rgb.height(),
                image::ExtendedColorType::Rgb8,
            )?;
        }
        ImageFormat::Webp | ImageFormat::Png => {
            DynamicImage::ImageRgba8(image.clone())
                .write_to(&mut bytes, format.image_crate_format())?;
        }
    }

    Ok(bytes.into_inner())
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
