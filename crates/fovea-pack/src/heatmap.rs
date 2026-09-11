use std::collections::HashMap;

use anyhow::{anyhow, Result};
use serde::Serialize;

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

pub fn build_heatmap_from_centroids(
    centroids: &[(f32, f32)],
    width: u32,
    height: u32,
    options: HeatmapBuildOptions,
) -> Result<InMemoryHeatmap> {
    validate_build_options(&options)?;
    let mut levels = build_density_levels(&options, centroids, width, height);
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
        width,
        height,
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
    centroids: &[(f32, f32)],
    width: u32,
    height: u32,
) -> Vec<HeatmapLevel> {
    let width = width.div_ceil(options.bin_size).max(1);
    let height = height.div_ceil(options.bin_size).max(1);
    let mut base = HeatmapLevel {
        index: 0,
        width,
        height,
        downsample: f64::from(options.bin_size),
        values: vec![0.0; width as usize * height as usize],
    };

    for centroid in centroids {
        let x = (centroid.0.max(0.0) as u32 / options.bin_size).min(width - 1);
        let y = (centroid.1.max(0.0) as u32 / options.bin_size).min(height - 1);
        base.values[y as usize * width as usize + x as usize] += 1.0;
    }

    eprintln!(
        "fovea-pack: binned {} cell centroids into {}x{} in-memory heatmap",
        centroids.len(),
        width,
        height
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

    levels
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

fn validate_build_options(options: &HeatmapBuildOptions) -> Result<()> {
    if options.bin_size == 0 {
        return Err(anyhow!("heatmap bin size must be greater than zero"));
    }

    if options.tile_size == 0 {
        return Err(anyhow!("heatmap tile size must be greater than zero"));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{build_density_levels, HeatmapBuildOptions};

    #[test]
    fn bins_centroids_and_clamps_to_edges() {
        let options = HeatmapBuildOptions {
            id: "test".to_string(),
            bin_size: 128,
            tile_size: 256,
        };
        // 300x200 px -> 3x2 bins.
        let centroids = [
            (0.0, 0.0),
            (127.9, 127.9),
            (128.0, 0.0),
            (-5.0, 150.0),
            (299.0, 199.0),
            (10_000.0, 10_000.0),
        ];
        let levels = build_density_levels(&options, &centroids, 300, 200);

        assert_eq!((levels[0].width, levels[0].height), (3, 2));
        assert_eq!(levels[0].values, vec![2.0, 1.0, 0.0, 1.0, 0.0, 2.0]);
        assert_eq!(
            levels.last().map(|level| (level.width, level.height)),
            Some((1, 1))
        );
    }
}
