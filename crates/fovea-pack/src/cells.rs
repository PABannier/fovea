use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::Write,
    path::PathBuf,
};

use anyhow::{anyhow, Context, Result};
use prost::Message;
use serde::Serialize;

#[derive(Clone, Debug)]
pub struct CellLoadOptions {
    pub proto_path: PathBuf,
    pub id: String,
    pub chunk_size: u32,
    pub max_vertices_per_cell: u16,
}

#[derive(Clone, Debug)]
pub struct InMemoryCells {
    pub manifest_json: String,
    pub chunks: HashMap<(u32, u32), Vec<u8>>,
    pub(crate) manifest: CellManifest,
}

#[derive(Clone, Debug)]
struct CellRecord {
    cell_id: u64,
    class_id: u16,
    confidence: f32,
    centroid: Point,
    bbox: BBox,
    polygon: Vec<Point>,
}

#[derive(Clone, Copy, Debug, Default)]
struct Point {
    x: f32,
    y: f32,
}

#[derive(Clone, Copy, Debug)]
struct BBox {
    min_x: f32,
    min_y: f32,
    max_x: f32,
    max_y: f32,
}

impl BBox {
    fn from_points(points: &[Point]) -> Option<Self> {
        let first = points.first().copied()?;
        let mut bbox = Self {
            min_x: first.x,
            min_y: first.y,
            max_x: first.x,
            max_y: first.y,
        };

        for point in points.iter().copied().skip(1) {
            bbox.min_x = bbox.min_x.min(point.x);
            bbox.min_y = bbox.min_y.min(point.y);
            bbox.max_x = bbox.max_x.max(point.x);
            bbox.max_y = bbox.max_y.max(point.y);
        }

        Some(bbox)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct ChunkKey {
    x: u32,
    y: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CellManifest {
    pub(crate) schema: String,
    pub(crate) version: String,
    pub(crate) id: String,
    pub(crate) source_format: String,
    pub(crate) slide_id: String,
    pub(crate) slide_path: String,
    pub(crate) mpp: f32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) chunk_width: u32,
    pub(crate) chunk_height: u32,
    pub(crate) chunk_cols: u32,
    pub(crate) chunk_rows: u32,
    pub(crate) cell_count: u64,
    pub(crate) polygon_vertex_count: u64,
    pub(crate) max_vertices_per_cell: u16,
    pub(crate) classes: Vec<CellClassManifest>,
    pub(crate) chunks: Vec<CellChunkManifest>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CellClassManifest {
    pub(crate) id: u16,
    pub(crate) name: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CellChunkManifest {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) path: String,
    pub(crate) cell_count: u32,
    pub(crate) polygon_vertex_count: u32,
    pub(crate) byte_size: u64,
    pub(crate) bbox: ChunkBBoxManifest,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChunkBBoxManifest {
    pub(crate) min_x: f32,
    pub(crate) min_y: f32,
    pub(crate) max_x: f32,
    pub(crate) max_y: f32,
}

pub fn load_cells_protobuf(options: CellLoadOptions) -> Result<InMemoryCells> {
    validate_load_options(&options)?;
    let bytes = fs::read(&options.proto_path)
        .with_context(|| format!("failed to read {}", options.proto_path.display()))?;

    eprintln!(
        "fovea-pack: reading protobuf {} ({:.2} MB)",
        options.proto_path.display(),
        bytes.len() as f64 / (1024.0 * 1024.0)
    );

    let packed = match new_proto::SlideSegmentationData::decode(bytes.as_slice()) {
        Ok(data) if looks_like_new_proto(&data) => {
            eprintln!("fovea-pack: decoded histotyper_v2 new_cell_masks protobuf");
            decode_new_proto_data(&data, &options)?
        }
        _ => {
            let data = legacy_proto::SlideSegmentationData::decode(bytes.as_slice())
                .with_context(|| format!("failed to decode {}", options.proto_path.display()))?;
            eprintln!("fovea-pack: decoded legacy cell_masks protobuf");
            decode_legacy_proto_data(&data, &options)?
        }
    };

    packed.into_memory()
}

struct PackedCells {
    manifest: CellManifest,
    chunks: HashMap<(u32, u32), Vec<u8>>,
}

impl PackedCells {
    fn into_memory(self) -> Result<InMemoryCells> {
        let manifest_json = serde_json::to_string_pretty(&self.manifest)?;
        Ok(InMemoryCells {
            manifest_json,
            chunks: self.chunks,
            manifest: self.manifest,
        })
    }
}

fn looks_like_new_proto(data: &new_proto::SlideSegmentationData) -> bool {
    !data.tiles.is_empty() || !data.cell_class_names.is_empty() || data.tile_size != 0
}

fn decode_new_proto_data(
    data: &new_proto::SlideSegmentationData,
    options: &CellLoadOptions,
) -> Result<PackedCells> {
    let mut chunks: BTreeMap<ChunkKey, Vec<CellRecord>> = BTreeMap::new();
    let mut width = 0.0_f32;
    let mut height = 0.0_f32;
    let mut invalid_polygons = 0_u64;
    let mut vertex_count = 0_u64;
    let mut cell_id = 0_u64;
    let scale_factor = if data.max_level >= data.level {
        (1_u64 << (data.max_level - data.level).min(32)) as f32
    } else {
        1.0 / (1_u64 << (data.level - data.max_level).min(32)) as f32
    };

    for tile in &data.tiles {
        let cells = decode_new_cells_blob(tile, data, scale_factor).with_context(|| {
            format!(
                "failed to decode cells_blob for tile ({}, {})",
                tile.x, tile.y
            )
        })?;

        width = width.max((tile.x as f32 + 1.0) * data.tile_size as f32 * scale_factor);
        height = height.max((tile.y as f32 + 1.0) * data.tile_size as f32 * scale_factor);

        for mut cell in cells {
            if cell.polygon.len() < 3 {
                invalid_polygons += 1;
                continue;
            }

            if options.max_vertices_per_cell > 0
                && cell.polygon.len() > options.max_vertices_per_cell as usize
            {
                cell.polygon =
                    simplify_by_stride(&cell.polygon, options.max_vertices_per_cell as usize);
                cell.bbox = BBox::from_points(&cell.polygon).unwrap_or(cell.bbox);
            }

            cell.cell_id = cell_id;
            cell_id = cell_id.wrapping_add(1);
            width = width.max(cell.bbox.max_x.max(cell.centroid.x));
            height = height.max(cell.bbox.max_y.max(cell.centroid.y));
            vertex_count += cell.polygon.len() as u64;
            push_cell_to_chunk(&mut chunks, options.chunk_size, cell);
        }
    }

    if invalid_polygons > 0 {
        eprintln!("fovea-pack: skipped {invalid_polygons} invalid cell polygons");
    }

    let classes = data
        .cell_class_names
        .iter()
        .enumerate()
        .map(|(id, name)| CellClassManifest {
            id: id.min(u16::MAX as usize) as u16,
            name: name.clone(),
        })
        .collect();

    build_packed_cells(
        chunks,
        CellManifestInput {
            id: options.id.clone(),
            source_format: "histotyper_v2.SlideSegmentationData protobuf".to_string(),
            slide_id: data.slide_id.clone(),
            slide_path: data.slide_path.clone(),
            mpp: data.mpp,
            width,
            height,
            chunk_size: options.chunk_size,
            max_vertices_per_cell: options.max_vertices_per_cell,
            classes,
            polygon_vertex_count: vertex_count,
        },
    )
}

fn decode_legacy_proto_data(
    data: &legacy_proto::SlideSegmentationData,
    options: &CellLoadOptions,
) -> Result<PackedCells> {
    let mut class_ids = ClassIds::default();
    let mut chunks: BTreeMap<ChunkKey, Vec<CellRecord>> = BTreeMap::new();
    let mut width = 0.0_f32;
    let mut height = 0.0_f32;
    let mut invalid_polygons = 0_u64;
    let mut vertex_count = 0_u64;

    for tile in &data.tiles {
        let tile_x = tile.x.unwrap_or_default();
        let tile_y = tile.y.unwrap_or_default();
        let tile_width = tile.width.unwrap_or_default().max(0) as f32;
        let tile_height = tile.height.unwrap_or_default().max(0) as f32;
        width = width.max(tile_x + tile_width);
        height = height.max(tile_y + tile_height);

        for mask in &tile.masks {
            let Some(cell) = convert_mask(mask, &mut class_ids, options.max_vertices_per_cell)
            else {
                invalid_polygons += 1;
                continue;
            };

            width = width.max(cell.bbox.max_x.max(cell.centroid.x));
            height = height.max(cell.bbox.max_y.max(cell.centroid.y));
            vertex_count += cell.polygon.len() as u64;
            push_cell_to_chunk(&mut chunks, options.chunk_size, cell);
        }
    }

    if invalid_polygons > 0 {
        eprintln!("fovea-pack: skipped {invalid_polygons} invalid cell polygons");
    }

    build_packed_cells(
        chunks,
        CellManifestInput {
            id: options.id.clone(),
            source_format: "histotyper.SlideSegmentationData protobuf".to_string(),
            slide_id: data.slide_id.clone().unwrap_or_default(),
            slide_path: data.slide_path.clone().unwrap_or_default(),
            mpp: data.mpp.unwrap_or_default(),
            width,
            height,
            chunk_size: options.chunk_size,
            max_vertices_per_cell: options.max_vertices_per_cell,
            classes: class_ids.into_manifest(),
            polygon_vertex_count: vertex_count,
        },
    )
}

fn push_cell_to_chunk(
    chunks: &mut BTreeMap<ChunkKey, Vec<CellRecord>>,
    chunk_size: u32,
    cell: CellRecord,
) {
    let chunk_size = chunk_size as f32;
    let chunk_x = (cell.centroid.x.max(0.0) / chunk_size).floor() as u32;
    let chunk_y = (cell.centroid.y.max(0.0) / chunk_size).floor() as u32;
    chunks
        .entry(ChunkKey {
            x: chunk_x,
            y: chunk_y,
        })
        .or_default()
        .push(cell);
}

struct CellManifestInput {
    id: String,
    source_format: String,
    slide_id: String,
    slide_path: String,
    mpp: f32,
    width: f32,
    height: f32,
    chunk_size: u32,
    max_vertices_per_cell: u16,
    classes: Vec<CellClassManifest>,
    polygon_vertex_count: u64,
}

fn build_packed_cells(
    chunks: BTreeMap<ChunkKey, Vec<CellRecord>>,
    input: CellManifestInput,
) -> Result<PackedCells> {
    let width = input.width.ceil().max(1.0) as u32;
    let height = input.height.ceil().max(1.0) as u32;
    let chunk_cols = width.div_ceil(input.chunk_size);
    let chunk_rows = height.div_ceil(input.chunk_size);
    let mut chunk_manifests = Vec::with_capacity(chunks.len());
    let mut chunk_bytes = HashMap::with_capacity(chunks.len());

    for (key, cells) in chunks {
        let file_name = format!("{}_{}.fovc", key.x, key.y);
        let relative_path = format!("chunks/{file_name}");
        let (bytes, stats) = encode_chunk(key, input.chunk_size, &cells)?;

        chunk_manifests.push(CellChunkManifest {
            x: key.x,
            y: key.y,
            path: relative_path,
            cell_count: cells.len() as u32,
            polygon_vertex_count: stats.polygon_vertex_count,
            byte_size: stats.byte_size,
            bbox: ChunkBBoxManifest {
                min_x: stats.bbox.min_x,
                min_y: stats.bbox.min_y,
                max_x: stats.bbox.max_x,
                max_y: stats.bbox.max_y,
            },
        });
        chunk_bytes.insert((key.x, key.y), bytes);
    }

    Ok(PackedCells {
        manifest: CellManifest {
            schema: "fovea.cells".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            id: input.id,
            source_format: input.source_format,
            slide_id: input.slide_id,
            slide_path: input.slide_path,
            mpp: input.mpp,
            width,
            height,
            chunk_width: input.chunk_size,
            chunk_height: input.chunk_size,
            chunk_cols,
            chunk_rows,
            cell_count: chunk_manifests
                .iter()
                .map(|chunk| u64::from(chunk.cell_count))
                .sum(),
            polygon_vertex_count: input.polygon_vertex_count,
            max_vertices_per_cell: input.max_vertices_per_cell,
            classes: input.classes,
            chunks: chunk_manifests,
        },
        chunks: chunk_bytes,
    })
}

fn convert_mask(
    mask: &legacy_proto::SegmentationPolygon,
    class_ids: &mut ClassIds,
    max_vertices_per_cell: u16,
) -> Option<CellRecord> {
    let mut polygon = Vec::with_capacity(mask.coordinates.len());

    for point in &mask.coordinates {
        let x = point.x?;
        let y = point.y?;

        if !x.is_finite() || !y.is_finite() {
            return None;
        }

        polygon.push(Point { x, y });
    }

    polygon.dedup_by(|left, right| left.x == right.x && left.y == right.y);

    if polygon.len() < 3 {
        return None;
    }

    if let (Some(first), Some(last)) = (polygon.first().copied(), polygon.last().copied()) {
        if first.x == last.x && first.y == last.y {
            polygon.pop();
        }
    }

    if polygon.len() < 3 {
        return None;
    }

    if max_vertices_per_cell > 0 && polygon.len() > max_vertices_per_cell as usize {
        polygon = simplify_by_stride(&polygon, max_vertices_per_cell as usize);
    }

    let bbox = BBox::from_points(&polygon)?;
    let centroid = mask
        .centroid
        .as_ref()
        .and_then(|point| {
            Some(Point {
                x: point.x?,
                y: point.y?,
            })
        })
        .filter(|point| point.x.is_finite() && point.y.is_finite())
        .unwrap_or_else(|| polygon_centroid(&polygon));

    Some(CellRecord {
        cell_id: mask.cell_id.unwrap_or_default().max(0) as u64,
        class_id: class_ids.id_for(mask.cell_type.as_deref().unwrap_or("unknown")),
        confidence: mask.confidence.unwrap_or_default(),
        centroid,
        bbox,
        polygon,
    })
}

fn decode_new_cells_blob(
    tile: &new_proto::TileSegmentationData,
    data: &new_proto::SlideSegmentationData,
    scale_factor: f32,
) -> Result<Vec<CellRecord>> {
    if tile.cells_blob.is_empty() {
        return Ok(Vec::new());
    }

    let decompressed = zstd::stream::decode_all(tile.cells_blob.as_slice())
        .context("failed to zstd-decompress cells_blob")?;
    let mut reader = CellBlobReader::new(&decompressed);
    let cell_count = reader.read_u16()? as usize;
    let tile_origin_x = tile.x as f32 * data.tile_size as f32;
    let tile_origin_y = tile.y as f32 * data.tile_size as f32;
    let mut cells = Vec::with_capacity(cell_count);

    for _ in 0..cell_count {
        let class_id = u16::from(reader.read_u8()?);
        let confidence = f32::from(reader.read_u8()?) / 255.0;
        let centroid_x = f32::from(reader.read_i16()?);
        let centroid_y = f32::from(reader.read_i16()?);
        let vertex_count = reader.read_u8()? as usize;
        let mut polygon = Vec::with_capacity(vertex_count);

        for _ in 0..vertex_count {
            let x = (f32::from(reader.read_i16()?) + tile_origin_x) * scale_factor;
            let y = (f32::from(reader.read_i16()?) + tile_origin_y) * scale_factor;
            polygon.push(Point { x, y });
        }

        polygon.dedup_by(|left, right| left.x == right.x && left.y == right.y);

        if let (Some(first), Some(last)) = (polygon.first().copied(), polygon.last().copied()) {
            if first.x == last.x && first.y == last.y {
                polygon.pop();
            }
        }

        let Some(bbox) = BBox::from_points(&polygon) else {
            continue;
        };
        let centroid = Point {
            x: (centroid_x + tile_origin_x) * scale_factor,
            y: (centroid_y + tile_origin_y) * scale_factor,
        };

        cells.push(CellRecord {
            cell_id: 0,
            class_id,
            confidence,
            centroid,
            bbox,
            polygon,
        });
    }

    Ok(cells)
}

struct CellBlobReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> CellBlobReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn read_bytes(&mut self, len: usize) -> Result<&'a [u8]> {
        if self.offset + len > self.bytes.len() {
            return Err(anyhow!("cells_blob ended unexpectedly"));
        }

        let start = self.offset;
        self.offset += len;
        Ok(&self.bytes[start..self.offset])
    }

    fn read_u8(&mut self) -> Result<u8> {
        Ok(self.read_bytes(1)?[0])
    }

    fn read_u16(&mut self) -> Result<u16> {
        let bytes = self.read_bytes(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn read_i16(&mut self) -> Result<i16> {
        let bytes = self.read_bytes(2)?;
        Ok(i16::from_le_bytes([bytes[0], bytes[1]]))
    }
}

fn polygon_centroid(points: &[Point]) -> Point {
    let mut area2 = 0.0_f32;
    let mut cx = 0.0_f32;
    let mut cy = 0.0_f32;

    for index in 0..points.len() {
        let a = points[index];
        let b = points[(index + 1) % points.len()];
        let cross = a.x * b.y - b.x * a.y;
        area2 += cross;
        cx += (a.x + b.x) * cross;
        cy += (a.y + b.y) * cross;
    }

    if area2.abs() <= f32::EPSILON {
        let inv = 1.0 / points.len() as f32;
        return Point {
            x: points.iter().map(|point| point.x).sum::<f32>() * inv,
            y: points.iter().map(|point| point.y).sum::<f32>() * inv,
        };
    }

    Point {
        x: cx / (3.0 * area2),
        y: cy / (3.0 * area2),
    }
}

fn simplify_by_stride(points: &[Point], max_vertices: usize) -> Vec<Point> {
    if points.len() <= max_vertices {
        return points.to_vec();
    }

    let stride = points.len() as f32 / max_vertices as f32;
    (0..max_vertices)
        .map(|index| points[(index as f32 * stride).floor() as usize])
        .collect()
}

#[derive(Default)]
struct ClassIds {
    by_name: HashMap<String, u16>,
}

impl ClassIds {
    fn id_for(&mut self, name: &str) -> u16 {
        if let Some(id) = self.by_name.get(name) {
            return *id;
        }

        let id = self.by_name.len().min(u16::MAX as usize) as u16;
        self.by_name.insert(name.to_string(), id);
        id
    }

    fn into_manifest(self) -> Vec<CellClassManifest> {
        let mut classes: Vec<_> = self
            .by_name
            .into_iter()
            .map(|(name, id)| CellClassManifest { id, name })
            .collect();
        classes.sort_by_key(|class| class.id);
        classes
    }
}

#[derive(Debug)]
struct ChunkStats {
    polygon_vertex_count: u32,
    byte_size: u64,
    bbox: BBox,
}

fn encode_chunk(
    key: ChunkKey,
    chunk_size: u32,
    cells: &[CellRecord],
) -> Result<(Vec<u8>, ChunkStats)> {
    let mut writer = Vec::new();
    let origin_x = (key.x * chunk_size) as f32;
    let origin_y = (key.y * chunk_size) as f32;
    let polygon_vertex_count = cells
        .iter()
        .map(|cell| cell.polygon.len())
        .sum::<usize>()
        .min(u32::MAX as usize) as u32;
    let mut bbox = BBox {
        min_x: f32::MAX,
        min_y: f32::MAX,
        max_x: f32::MIN,
        max_y: f32::MIN,
    };

    writer.write_all(b"FOVC")?;
    write_u32(&mut writer, 1)?;
    write_u32(&mut writer, key.x)?;
    write_u32(&mut writer, key.y)?;
    write_f32(&mut writer, origin_x)?;
    write_f32(&mut writer, origin_y)?;
    write_f32(&mut writer, chunk_size as f32)?;
    write_f32(&mut writer, chunk_size as f32)?;
    write_u32(&mut writer, cells.len().min(u32::MAX as usize) as u32)?;
    write_u32(&mut writer, polygon_vertex_count)?;

    let mut vertex_offset = 0_u32;
    for cell in cells {
        bbox.min_x = bbox.min_x.min(cell.bbox.min_x);
        bbox.min_y = bbox.min_y.min(cell.bbox.min_y);
        bbox.max_x = bbox.max_x.max(cell.bbox.max_x);
        bbox.max_y = bbox.max_y.max(cell.bbox.max_y);

        write_u64(&mut writer, cell.cell_id)?;
        write_u16(&mut writer, cell.class_id)?;
        write_u16(
            &mut writer,
            cell.polygon.len().min(u16::MAX as usize) as u16,
        )?;
        write_f32(&mut writer, cell.confidence)?;
        write_u16(&mut writer, quantize(cell.centroid.x, origin_x, chunk_size))?;
        write_u16(&mut writer, quantize(cell.centroid.y, origin_y, chunk_size))?;
        write_u32(&mut writer, vertex_offset)?;
        write_u16(&mut writer, quantize(cell.bbox.min_x, origin_x, chunk_size))?;
        write_u16(&mut writer, quantize(cell.bbox.min_y, origin_y, chunk_size))?;
        write_u16(&mut writer, quantize(cell.bbox.max_x, origin_x, chunk_size))?;
        write_u16(&mut writer, quantize(cell.bbox.max_y, origin_y, chunk_size))?;
        vertex_offset = vertex_offset.saturating_add(cell.polygon.len() as u32);
    }

    for cell in cells {
        for point in &cell.polygon {
            write_u16(&mut writer, quantize(point.x, origin_x, chunk_size))?;
            write_u16(&mut writer, quantize(point.y, origin_y, chunk_size))?;
        }
    }

    let byte_size = writer.len() as u64;
    Ok((
        writer,
        ChunkStats {
            polygon_vertex_count,
            byte_size,
            bbox,
        },
    ))
}

impl InMemoryCells {
    pub(crate) fn width(&self) -> u32 {
        self.manifest.width
    }

    pub(crate) fn height(&self) -> u32 {
        self.manifest.height
    }

    pub(crate) fn chunk_width(&self) -> u32 {
        self.manifest.chunk_width
    }

    pub(crate) fn chunk_height(&self) -> u32 {
        self.manifest.chunk_height
    }

    pub(crate) fn chunks(&self) -> &[CellChunkManifest] {
        &self.manifest.chunks
    }
}

fn quantize(value: f32, origin: f32, chunk_size: u32) -> u16 {
    let normalized = ((value - origin) / chunk_size as f32).clamp(0.0, 1.0);
    (normalized * u16::MAX as f32).round() as u16
}

fn write_u16(writer: &mut dyn Write, value: u16) -> Result<()> {
    writer.write_all(&value.to_le_bytes())?;
    Ok(())
}

fn write_u32(writer: &mut dyn Write, value: u32) -> Result<()> {
    writer.write_all(&value.to_le_bytes())?;
    Ok(())
}

fn write_u64(writer: &mut dyn Write, value: u64) -> Result<()> {
    writer.write_all(&value.to_le_bytes())?;
    Ok(())
}

fn write_f32(writer: &mut dyn Write, value: f32) -> Result<()> {
    writer.write_all(&value.to_le_bytes())?;
    Ok(())
}

fn validate_load_options(options: &CellLoadOptions) -> Result<()> {
    if !options.proto_path.exists() {
        return Err(anyhow!(
            "input protobuf does not exist: {}",
            options.proto_path.display()
        ));
    }

    if options.chunk_size == 0 {
        return Err(anyhow!("--chunk-size must be greater than zero"));
    }

    Ok(())
}
mod new_proto {
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct TileSegmentationData {
        #[prost(uint32, tag = "1")]
        pub x: u32,
        #[prost(uint32, tag = "2")]
        pub y: u32,
        #[prost(bytes = "vec", tag = "3")]
        pub cells_blob: Vec<u8>,
        #[prost(bytes = "vec", tag = "4")]
        pub tissue_blob: Vec<u8>,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct SlideSegmentationData {
        #[prost(string, tag = "1")]
        pub slide_id: String,
        #[prost(string, tag = "2")]
        pub slide_path: String,
        #[prost(float, tag = "3")]
        pub mpp: f32,
        #[prost(uint32, tag = "4")]
        pub max_level: u32,
        #[prost(uint32, tag = "5")]
        pub level: u32,
        #[prost(uint32, tag = "6")]
        pub tile_size: u32,
        #[prost(string, tag = "7")]
        pub cell_model_name: String,
        #[prost(string, tag = "8")]
        pub tissue_model_name: String,
        #[prost(string, repeated, tag = "9")]
        pub cell_class_names: Vec<String>,
        #[prost(string, repeated, tag = "10")]
        pub tissue_class_names: Vec<String>,
        #[prost(message, repeated, tag = "11")]
        pub tiles: Vec<TileSegmentationData>,
    }
}

mod legacy_proto {
    use std::collections::HashMap;

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct SegmentationPolygon {
        #[prost(int32, optional, tag = "1")]
        pub cell_id: Option<i32>,
        #[prost(string, optional, tag = "2")]
        pub cell_type: Option<String>,
        #[prost(float, optional, tag = "3")]
        pub confidence: Option<f32>,
        #[prost(message, repeated, tag = "4")]
        pub coordinates: Vec<Point>,
        #[prost(message, optional, tag = "5")]
        pub centroid: Option<Point>,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Point {
        #[prost(float, optional, tag = "1")]
        pub x: Option<f32>,
        #[prost(float, optional, tag = "2")]
        pub y: Option<f32>,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct TileSegmentationData {
        #[prost(string, optional, tag = "1")]
        pub tile_id: Option<String>,
        #[prost(int32, optional, tag = "2")]
        pub level: Option<i32>,
        #[prost(float, optional, tag = "3")]
        pub x: Option<f32>,
        #[prost(float, optional, tag = "4")]
        pub y: Option<f32>,
        #[prost(int32, optional, tag = "5")]
        pub width: Option<i32>,
        #[prost(int32, optional, tag = "6")]
        pub height: Option<i32>,
        #[prost(message, repeated, tag = "7")]
        pub masks: Vec<SegmentationPolygon>,
        #[prost(message, optional, tag = "8")]
        pub tissue_segmentation_map: Option<TissueSegmentationMap>,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct TissueSegmentationMap {
        #[prost(bytes, optional, tag = "1")]
        pub data: Option<Vec<u8>>,
        #[prost(int32, optional, tag = "2")]
        pub width: Option<i32>,
        #[prost(int32, optional, tag = "3")]
        pub height: Option<i32>,
        #[prost(string, optional, tag = "4")]
        pub dtype: Option<String>,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct SlideSegmentationData {
        #[prost(string, optional, tag = "1")]
        pub slide_id: Option<String>,
        #[prost(string, optional, tag = "2")]
        pub slide_path: Option<String>,
        #[prost(float, optional, tag = "3")]
        pub mpp: Option<f32>,
        #[prost(int32, optional, tag = "4")]
        pub max_level: Option<i32>,
        #[prost(string, optional, tag = "5")]
        pub cell_model_name: Option<String>,
        #[prost(string, optional, tag = "6")]
        pub tissue_model_name: Option<String>,
        #[prost(message, repeated, tag = "8")]
        pub tiles: Vec<TileSegmentationData>,
        #[prost(map = "int32, string", tag = "9")]
        pub tissue_class_mapping: HashMap<i32, String>,
    }
}

#[cfg(test)]
mod tests {
    use super::{polygon_centroid, quantize, Point};

    #[test]
    fn quantize_clamps_to_chunk() {
        assert_eq!(quantize(-10.0, 0.0, 4096), 0);
        assert_eq!(quantize(4096.0, 0.0, 4096), u16::MAX);
    }

    #[test]
    fn computes_polygon_centroid() {
        let centroid = polygon_centroid(&[
            Point { x: 0.0, y: 0.0 },
            Point { x: 2.0, y: 0.0 },
            Point { x: 2.0, y: 2.0 },
            Point { x: 0.0, y: 2.0 },
        ]);
        assert!((centroid.x - 1.0).abs() < 0.001);
        assert!((centroid.y - 1.0).abs() < 0.001);
    }
}
