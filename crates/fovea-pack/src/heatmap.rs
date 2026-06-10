use std::collections::HashMap;

use anyhow::{anyhow, Result};
use serde::Serialize;

use crate::cells::InMemoryCells;

#[derive(Clone, Debug)]
pub struct HeatmapBuildOptions {
    pub id: String,
    pub bin_size: u32,
    pub tile_size: u32,
}

#[derive(Clone, Debug)]
pub struct InMemoryHeatmap {
    pub manifest_json: String,
    pub tiles: HashMap<(u32, u32, u32), Vec<u8>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HeatmapManifest {
    schema: String,
    version: String,
    id: String,
    source_format: String,
    width: u32,
    height: u32,
    tile_size: u32,
    value_min: f32,
    value_max: f32,
    levels: Vec<HeatmapLevelManifest>,
    tiles: Vec<HeatmapTileManifest>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HeatmapLevelManifest {
    index: u32,
    width: u32,
    height: u32,
    downsample: f64,
    tile_cols: u32,
    tile_rows: u32,
    tile_count: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HeatmapTileManifest {
    level: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    path: String,
    byte_size: u64,
}

#[derive(Clone)]
struct HeatmapLevel {
    index: u32,
    width: u32,
    height: u32,
    downsample: f64,
    values: Vec<f32>,
}

pub fn build_heatmap_from_cells(
    cells: &InMemoryCells,
    options: HeatmapBuildOptions,
) -> Result<InMemoryHeatmap> {
    validate_build_options(&options)?;
    let mut levels = build_density_levels(&options, cells)?;
    let value_max = levels
        .iter()
        .flat_map(|level| level.values.iter().copied())
        .fold(0.0_f32, f32::max)
        .max(1.0);
    let (tile_manifests, tiles) = build_heatmap_tiles(&options, &levels, value_max);
    let level_manifests = levels
        .drain(..)
        .map(|level| {
            let tile_cols = level.width.div_ceil(options.tile_size);
            let tile_rows = level.height.div_ceil(options.tile_size);
            HeatmapLevelManifest {
                index: level.index,
                width: level.width,
                height: level.height,
                downsample: level.downsample,
                tile_cols,
                tile_rows,
                tile_count: tile_cols * tile_rows,
            }
        })
        .collect();
    let manifest = HeatmapManifest {
        schema: "fovea.heatmap".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        id: options.id,
        source_format: "fovea.cell centroid density".to_string(),
        width: cells.width(),
        height: cells.height(),
        tile_size: options.tile_size,
        value_min: 0.0,
        value_max,
        levels: level_manifests,
        tiles: tile_manifests,
    };

    Ok(InMemoryHeatmap {
        manifest_json: serde_json::to_string_pretty(&manifest)?,
        tiles,
    })
}

fn build_density_levels(
    options: &HeatmapBuildOptions,
    cells: &InMemoryCells,
) -> Result<Vec<HeatmapLevel>> {
    let width = cells.width().div_ceil(options.bin_size).max(1);
    let height = cells.height().div_ceil(options.bin_size).max(1);
    let mut base = HeatmapLevel {
        index: 0,
        width,
        height,
        downsample: f64::from(options.bin_size),
        values: vec![0.0; width as usize * height as usize],
    };
    let mut cell_count = 0_u64;

    for chunk in cells.chunks() {
        let Some(bytes) = cells.chunks.get(&(chunk.x, chunk.y)) else {
            continue;
        };
        let centroids = read_cell_chunk_centroids(
            bytes,
            chunk.x,
            chunk.y,
            cells.chunk_width(),
            cells.chunk_height(),
        )?;

        for centroid in centroids {
            let x = (centroid.0.max(0.0) as u32 / options.bin_size).min(width - 1);
            let y = (centroid.1.max(0.0) as u32 / options.bin_size).min(height - 1);
            base.values[y as usize * width as usize + x as usize] += 1.0;
            cell_count += 1;
        }
    }

    eprintln!(
        "fovea-pack: binned {cell_count} cell centroids into {}x{} in-memory heatmap",
        width, height
    );

    let mut levels = vec![base];
    while levels
        .last()
        .map(|level| level.width > 1 || level.height > 1)
        == Some(true)
    {
        let previous = levels.last().expect("base heatmap level exists");
        levels.push(downsample_level(previous));
    }

    Ok(levels)
}

fn downsample_level(previous: &HeatmapLevel) -> HeatmapLevel {
    let width = previous.width.div_ceil(2).max(1);
    let height = previous.height.div_ceil(2).max(1);
    let mut values = vec![0.0; width as usize * height as usize];

    for y in 0..height {
        for x in 0..width {
            let mut sum = 0.0_f32;
            let mut count = 0.0_f32;

            for dy in 0..2 {
                for dx in 0..2 {
                    let source_x = x * 2 + dx;
                    let source_y = y * 2 + dy;

                    if source_x < previous.width && source_y < previous.height {
                        sum += previous.values
                            [source_y as usize * previous.width as usize + source_x as usize];
                        count += 1.0;
                    }
                }
            }

            values[y as usize * width as usize + x as usize] = sum / count.max(1.0);
        }
    }

    HeatmapLevel {
        index: previous.index + 1,
        width,
        height,
        downsample: previous.downsample * 2.0,
        values,
    }
}

#[allow(clippy::type_complexity)]
fn build_heatmap_tiles(
    options: &HeatmapBuildOptions,
    levels: &[HeatmapLevel],
    value_max: f32,
) -> (Vec<HeatmapTileManifest>, HashMap<(u32, u32, u32), Vec<u8>>) {
    let mut manifests = Vec::new();
    let mut tiles = HashMap::new();

    for level in levels {
        let tile_cols = level.width.div_ceil(options.tile_size);
        let tile_rows = level.height.div_ceil(options.tile_size);

        for tile_y in 0..tile_rows {
            for tile_x in 0..tile_cols {
                let tile_width = (level.width - tile_x * options.tile_size).min(options.tile_size);
                let tile_height =
                    (level.height - tile_y * options.tile_size).min(options.tile_size);
                let file_name = format!("{tile_x}_{tile_y}.fovh");
                let bytes =
                    encode_heatmap_tile(level, tile_x, tile_y, options.tile_size, value_max);

                manifests.push(HeatmapTileManifest {
                    level: level.index,
                    x: tile_x,
                    y: tile_y,
                    width: tile_width,
                    height: tile_height,
                    path: format!("tiles/{}/{file_name}", level.index),
                    byte_size: bytes.len() as u64,
                });
                tiles.insert((level.index, tile_x, tile_y), bytes);
            }
        }
    }

    (manifests, tiles)
}

fn encode_heatmap_tile(
    level: &HeatmapLevel,
    tile_x: u32,
    tile_y: u32,
    tile_size: u32,
    value_max: f32,
) -> Vec<u8> {
    let tile_width = (level.width - tile_x * tile_size).min(tile_size);
    let tile_height = (level.height - tile_y * tile_size).min(tile_size);
    let mut bytes = vec![0_u8; tile_width as usize * tile_height as usize];

    for y in 0..tile_height {
        for x in 0..tile_width {
            let source_x = tile_x * tile_size + x;
            let source_y = tile_y * tile_size + y;
            let value = level.values[source_y as usize * level.width as usize + source_x as usize];
            bytes[y as usize * tile_width as usize + x as usize] =
                ((value / value_max).clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }

    bytes
}

fn read_cell_chunk_centroids(
    bytes: &[u8],
    expected_x: u32,
    expected_y: u32,
    chunk_width: u32,
    chunk_height: u32,
) -> Result<Vec<(f32, f32)>> {
    let mut reader = ByteReader::new(bytes);

    if reader.read_bytes(4)? != b"FOVC" {
        return Err(anyhow!("cell chunk has invalid magic"));
    }

    let version = reader.read_u32()?;
    if version != 1 {
        return Err(anyhow!("cell chunk has unsupported version {version}"));
    }

    let chunk_x = reader.read_u32()?;
    let chunk_y = reader.read_u32()?;
    if chunk_x != expected_x || chunk_y != expected_y {
        return Err(anyhow!("cell chunk coordinates do not match manifest"));
    }

    let origin_x = reader.read_f32()?;
    let origin_y = reader.read_f32()?;
    let encoded_width = reader.read_f32()?;
    let encoded_height = reader.read_f32()?;
    let cell_count = reader.read_u32()? as usize;
    let _polygon_vertex_count = reader.read_u32()?;
    let width = if encoded_width > 0.0 {
        encoded_width
    } else {
        chunk_width as f32
    };
    let height = if encoded_height > 0.0 {
        encoded_height
    } else {
        chunk_height as f32
    };
    let mut centroids = Vec::with_capacity(cell_count);

    for _ in 0..cell_count {
        let _cell_id = reader.read_u64()?;
        let _class_id = reader.read_u16()?;
        let _vertex_count = reader.read_u16()?;
        let _confidence = reader.read_f32()?;
        let centroid_x = reader.read_u16()?;
        let centroid_y = reader.read_u16()?;
        let _vertex_offset = reader.read_u32()?;
        let _bbox_min_x = reader.read_u16()?;
        let _bbox_min_y = reader.read_u16()?;
        let _bbox_max_x = reader.read_u16()?;
        let _bbox_max_y = reader.read_u16()?;

        centroids.push((
            dequantize(centroid_x, origin_x, width),
            dequantize(centroid_y, origin_y, height),
        ));
    }

    Ok(centroids)
}

fn dequantize(value: u16, origin: f32, size: f32) -> f32 {
    origin + (f32::from(value) / f32::from(u16::MAX)) * size
}

struct ByteReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> ByteReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn read_bytes(&mut self, len: usize) -> Result<&'a [u8]> {
        if self.offset + len > self.bytes.len() {
            return Err(anyhow!("heatmap source chunk ended unexpectedly"));
        }

        let start = self.offset;
        self.offset += len;
        Ok(&self.bytes[start..self.offset])
    }

    fn read_u16(&mut self) -> Result<u16> {
        let bytes = self.read_bytes(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn read_u32(&mut self) -> Result<u32> {
        let bytes = self.read_bytes(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_u64(&mut self) -> Result<u64> {
        let bytes = self.read_bytes(8)?;
        Ok(u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    fn read_f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.read_u32()?))
    }
}

fn validate_build_options(options: &HeatmapBuildOptions) -> Result<()> {
    if options.bin_size == 0 {
        return Err(anyhow!("heatmap bin size must be greater than zero"));
    }

    if options.tile_size == 0 {
        return Err(anyhow!("heatmap tile size must be greater than zero"));
    }

    Ok(())
}
