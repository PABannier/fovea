use std::{
    fs,
    io::{BufWriter, Read},
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug)]
pub struct HeatmapOverlayPackOptions {
    pub overlay_dir: PathBuf,
    pub out_dir: PathBuf,
    pub id: String,
    pub bin_size: u32,
    pub tile_size: u32,
    pub force: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CellOverlayManifest {
    width: u32,
    height: u32,
    chunk_width: u32,
    chunk_height: u32,
    chunks: Vec<CellChunkManifest>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CellChunkManifest {
    x: u32,
    y: u32,
    path: String,
}

#[derive(Debug, Serialize)]
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

#[derive(Debug, Serialize)]
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

#[derive(Debug, Serialize)]
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

pub fn pack_heatmap_from_cell_overlay(options: HeatmapOverlayPackOptions) -> Result<()> {
    validate_options(&options)?;
    let start = Instant::now();
    let output_dir = prepare_output_dir(&options.out_dir, options.force)?;
    let manifest_path = options.overlay_dir.join("manifest.json");
    let manifest: CellOverlayManifest =
        serde_json::from_reader(BufReaderFile::open(&manifest_path)?)
            .with_context(|| format!("failed to parse {}", manifest_path.display()))?;

    let mut levels = build_density_levels(&options, &manifest)?;
    let value_max = levels
        .iter()
        .flat_map(|level| level.values.iter().copied())
        .fold(0.0_f32, f32::max)
        .max(1.0);
    let tile_manifests = write_heatmap_tiles(&output_dir, &options, &levels, value_max)?;
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
    let heatmap_manifest = HeatmapManifest {
        schema: "fovea.heatmap".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        id: options.id.clone(),
        source_format: "fovea.cell-overlay centroid density".to_string(),
        width: manifest.width,
        height: manifest.height,
        tile_size: options.tile_size,
        value_min: 0.0,
        value_max,
        levels: level_manifests,
        tiles: tile_manifests,
    };

    write_manifest(&output_dir.join("manifest.json"), &heatmap_manifest)?;
    finalize_output_dir(&output_dir, &options.out_dir)?;

    eprintln!(
        "fovea-pack: done in {:.2}s, wrote heatmap {}x{} into {} tiles",
        start.elapsed().as_secs_f64(),
        manifest.width.div_ceil(options.bin_size),
        manifest.height.div_ceil(options.bin_size),
        heatmap_manifest.tiles.len()
    );

    Ok(())
}

fn build_density_levels(
    options: &HeatmapOverlayPackOptions,
    manifest: &CellOverlayManifest,
) -> Result<Vec<HeatmapLevel>> {
    let width = manifest.width.div_ceil(options.bin_size).max(1);
    let height = manifest.height.div_ceil(options.bin_size).max(1);
    let mut base = HeatmapLevel {
        index: 0,
        width,
        height,
        downsample: f64::from(options.bin_size),
        values: vec![0.0; width as usize * height as usize],
    };
    let mut cell_count = 0_u64;

    for chunk in &manifest.chunks {
        let path = options.overlay_dir.join(&chunk.path);
        let centroids = read_overlay_chunk_centroids(
            &path,
            chunk.x,
            chunk.y,
            manifest.chunk_width,
            manifest.chunk_height,
        )?;

        for centroid in centroids {
            let x = (centroid.0.max(0.0) as u32 / options.bin_size).min(width - 1);
            let y = (centroid.1.max(0.0) as u32 / options.bin_size).min(height - 1);
            base.values[y as usize * width as usize + x as usize] += 1.0;
            cell_count += 1;
        }
    }

    eprintln!(
        "fovea-pack: binned {cell_count} cell centroids into {}x{} heatmap",
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

fn write_heatmap_tiles(
    output_dir: &Path,
    options: &HeatmapOverlayPackOptions,
    levels: &[HeatmapLevel],
    value_max: f32,
) -> Result<Vec<HeatmapTileManifest>> {
    let mut tiles = Vec::new();

    for level in levels {
        let level_dir = output_dir.join("tiles").join(level.index.to_string());
        fs::create_dir_all(&level_dir)
            .with_context(|| format!("failed to create {}", level_dir.display()))?;
        let tile_cols = level.width.div_ceil(options.tile_size);
        let tile_rows = level.height.div_ceil(options.tile_size);

        for tile_y in 0..tile_rows {
            for tile_x in 0..tile_cols {
                let tile_width = (level.width - tile_x * options.tile_size).min(options.tile_size);
                let tile_height =
                    (level.height - tile_y * options.tile_size).min(options.tile_size);
                let file_name = format!("{tile_x}_{tile_y}.fovh");
                let path = level_dir.join(&file_name);
                let mut bytes = vec![0_u8; tile_width as usize * tile_height as usize];

                for y in 0..tile_height {
                    for x in 0..tile_width {
                        let source_x = tile_x * options.tile_size + x;
                        let source_y = tile_y * options.tile_size + y;
                        let value = level.values
                            [source_y as usize * level.width as usize + source_x as usize];
                        bytes[y as usize * tile_width as usize + x as usize] =
                            ((value / value_max).clamp(0.0, 1.0) * 255.0).round() as u8;
                    }
                }

                fs::write(&path, &bytes)
                    .with_context(|| format!("failed to write {}", path.display()))?;
                tiles.push(HeatmapTileManifest {
                    level: level.index,
                    x: tile_x,
                    y: tile_y,
                    width: tile_width,
                    height: tile_height,
                    path: format!("tiles/{}/{file_name}", level.index),
                    byte_size: bytes.len() as u64,
                });
            }
        }
    }

    Ok(tiles)
}

fn read_overlay_chunk_centroids(
    path: &Path,
    expected_x: u32,
    expected_y: u32,
    chunk_width: u32,
    chunk_height: u32,
) -> Result<Vec<(f32, f32)>> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut reader = ByteReader::new(&bytes);

    if reader.read_bytes(4)? != b"FOVC" {
        return Err(anyhow!(
            "{} has invalid overlay chunk magic",
            path.display()
        ));
    }

    let version = reader.read_u32()?;
    if version != 1 {
        return Err(anyhow!(
            "{} has unsupported chunk version {version}",
            path.display()
        ));
    }

    let chunk_x = reader.read_u32()?;
    let chunk_y = reader.read_u32()?;
    if chunk_x != expected_x || chunk_y != expected_y {
        return Err(anyhow!(
            "{} chunk coordinates do not match manifest",
            path.display()
        ));
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

struct BufReaderFile(fs::File);

impl BufReaderFile {
    fn open(path: &Path) -> Result<Self> {
        Ok(Self(fs::File::open(path).with_context(|| {
            format!("failed to open {}", path.display())
        })?))
    }
}

impl Read for BufReaderFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

fn validate_options(options: &HeatmapOverlayPackOptions) -> Result<()> {
    if !options.overlay_dir.join("manifest.json").exists() {
        return Err(anyhow!(
            "input overlay manifest does not exist: {}",
            options.overlay_dir.join("manifest.json").display()
        ));
    }

    if options.bin_size == 0 {
        return Err(anyhow!("--bin-size must be greater than zero"));
    }

    if options.tile_size == 0 {
        return Err(anyhow!("--tile-size must be greater than zero"));
    }

    Ok(())
}

fn prepare_output_dir(out_dir: &Path, force: bool) -> Result<PathBuf> {
    if out_dir.exists() {
        if force {
            fs::remove_dir_all(out_dir)
                .with_context(|| format!("failed to remove {}", out_dir.display()))?;
        } else {
            return Err(anyhow!(
                "output directory already exists: {} (use --force to replace it)",
                out_dir.display()
            ));
        }
    }

    let partial_dir = partial_output_dir(out_dir);
    if partial_dir.exists() {
        fs::remove_dir_all(&partial_dir)
            .with_context(|| format!("failed to remove stale {}", partial_dir.display()))?;
    }

    fs::create_dir_all(&partial_dir)
        .with_context(|| format!("failed to create {}", partial_dir.display()))?;
    Ok(partial_dir)
}

fn finalize_output_dir(partial_dir: &Path, out_dir: &Path) -> Result<()> {
    fs::rename(partial_dir, out_dir).with_context(|| {
        format!(
            "failed to move {} to {}",
            partial_dir.display(),
            out_dir.display()
        )
    })
}

fn partial_output_dir(out_dir: &Path) -> PathBuf {
    let name = out_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("heatmap");
    out_dir.with_file_name(format!("{name}.partial"))
}

fn write_manifest(path: &Path, manifest: &HeatmapManifest) -> Result<()> {
    let file =
        fs::File::create(path).with_context(|| format!("failed to create {}", path.display()))?;
    serde_json::to_writer_pretty(BufWriter::new(file), manifest)?;
    Ok(())
}
