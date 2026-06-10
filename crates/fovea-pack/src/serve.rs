use std::{
    collections::{HashMap, VecDeque},
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};

use anyhow::{anyhow, Context, Result};
use axum::{
    body::Body,
    extract::State,
    http::{header, Method, Request, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};

use crate::{
    cells::{load_cells_protobuf, CellLoadOptions, InMemoryCells},
    heatmap::{build_heatmap_from_cells, HeatmapBuildOptions, InMemoryHeatmap},
    manifest::{ImageFormat, Manifest},
    packer::{build_slide_manifest, encode_slide_tile, slide_tile_request},
    reader::{OpenSlideReader, SlideReader},
};

#[derive(Clone, Debug)]
pub struct ServeOptions {
    pub wsi_path: PathBuf,
    pub cells_protobuf_path: Option<PathBuf>,
    pub host: IpAddr,
    pub port: u16,
    pub tile_size: u32,
    pub image_format: ImageFormat,
    pub chunk_size: u32,
    pub max_vertices_per_cell: u16,
    pub heatmap: bool,
    pub heatmap_bin_size: u32,
    pub heatmap_tile_size: u32,
    pub tile_cache_mb: usize,
}

/// Per-slide source description, independent of any HTTP listener. This is the
/// subset of [`ServeOptions`] needed to prepare one slide's renderable sources,
/// so an embedding server (for example a multi-slide host) can reuse the
/// preparation and routing logic without binding its own socket.
#[derive(Clone, Debug)]
pub struct SourceOptions {
    pub wsi_path: PathBuf,
    pub cells_protobuf_path: Option<PathBuf>,
    pub tile_size: u32,
    pub image_format: ImageFormat,
    pub chunk_size: u32,
    pub max_vertices_per_cell: u16,
    pub heatmap: bool,
    pub heatmap_bin_size: u32,
    pub heatmap_tile_size: u32,
    pub tile_cache_mb: usize,
}

impl ServeOptions {
    fn source_options(&self) -> SourceOptions {
        SourceOptions {
            wsi_path: self.wsi_path.clone(),
            cells_protobuf_path: self.cells_protobuf_path.clone(),
            tile_size: self.tile_size,
            image_format: self.image_format,
            chunk_size: self.chunk_size,
            max_vertices_per_cell: self.max_vertices_per_cell,
            heatmap: self.heatmap,
            heatmap_bin_size: self.heatmap_bin_size,
            heatmap_tile_size: self.heatmap_tile_size,
            tile_cache_mb: self.tile_cache_mb,
        }
    }
}

/// Prepared, in-memory renderable sources for a single slide: the slide reader,
/// the precomputed slide manifest, the encoded-tile LRU cache, and the optional
/// cell chunks / density heatmap. Cheap to clone (everything is behind `Arc`).
/// Build one with [`prepare_sources`] and serve requests against it with
/// [`route_request`].
#[derive(Clone)]
pub struct SlideSources {
    reader: Arc<dyn SlideReader>,
    slide_manifest: Arc<Manifest>,
    slide_manifest_json: Arc<String>,
    image_format: ImageFormat,
    tile_cache: Arc<Mutex<TileCache>>,
    cells: Option<Arc<InMemoryCells>>,
    heatmap: Option<Arc<InMemoryHeatmap>>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct TileKey {
    level: u32,
    x: u32,
    y: u32,
}

struct TileCache {
    max_bytes: usize,
    current_bytes: usize,
    order: VecDeque<TileKey>,
    entries: HashMap<TileKey, Vec<u8>>,
}

impl TileCache {
    fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            current_bytes: 0,
            order: VecDeque::new(),
            entries: HashMap::new(),
        }
    }

    fn get(&mut self, key: TileKey) -> Option<Vec<u8>> {
        let bytes = self.entries.get(&key)?.clone();
        self.touch(key);
        Some(bytes)
    }

    fn insert(&mut self, key: TileKey, bytes: Vec<u8>) {
        if self.max_bytes == 0 {
            return;
        }

        if let Some(old) = self.entries.remove(&key) {
            self.current_bytes = self.current_bytes.saturating_sub(old.len());
            self.remove_from_order(key);
        }

        self.current_bytes = self.current_bytes.saturating_add(bytes.len());
        self.entries.insert(key, bytes);
        self.order.push_back(key);

        while self.current_bytes > self.max_bytes {
            let Some(old_key) = self.order.pop_front() else {
                break;
            };

            if let Some(old) = self.entries.remove(&old_key) {
                self.current_bytes = self.current_bytes.saturating_sub(old.len());
            }
        }
    }

    fn touch(&mut self, key: TileKey) {
        self.remove_from_order(key);
        self.order.push_back(key);
    }

    fn remove_from_order(&mut self, key: TileKey) {
        if let Some(index) = self.order.iter().position(|candidate| *candidate == key) {
            self.order.remove(index);
        }
    }
}

/// Prepare all renderable sources for a single slide: build the slide manifest,
/// load and chunk the cells, and build the density heatmap. Performs blocking
/// disk/CPU work (OpenSlide open, protobuf parse, heatmap build) but no network
/// I/O; callers that care about latency should run it off the request path. The
/// returned [`SlideSources`] is served by [`route_request`].
pub async fn prepare_sources(options: SourceOptions) -> Result<SlideSources> {
    if !options.wsi_path.exists() {
        return Err(anyhow!(
            "input WSI does not exist: {}",
            options.wsi_path.display()
        ));
    }

    if options.heatmap && options.cells_protobuf_path.is_none() {
        return Err(anyhow!("heatmap requires cells_protobuf_path"));
    }

    let reader: Arc<dyn SlideReader> = Arc::new(OpenSlideReader::open(&options.wsi_path)?);
    let slide_manifest = Arc::new(build_slide_manifest(
        reader.as_ref(),
        options.tile_size,
        options.image_format,
    )?);
    let slide_manifest_json = Arc::new(serde_json::to_string_pretty(slide_manifest.as_ref())?);

    let cells = if let Some(path) = &options.cells_protobuf_path {
        let start = Instant::now();
        let cells = load_cells_protobuf(CellLoadOptions {
            proto_path: path.clone(),
            id: "cells".to_string(),
            chunk_size: options.chunk_size,
            max_vertices_per_cell: options.max_vertices_per_cell,
        })?;
        eprintln!(
            "fovea-pack: prepared {} cell chunks in {:.2}s",
            cells.chunks.len(),
            start.elapsed().as_secs_f64()
        );
        Some(Arc::new(cells))
    } else {
        None
    };

    let heatmap = if options.heatmap {
        let cells = cells
            .as_ref()
            .expect("heatmap requires cells checked above");
        let start = Instant::now();
        let heatmap = build_heatmap_from_cells(
            cells,
            HeatmapBuildOptions {
                id: "cell_density".to_string(),
                bin_size: options.heatmap_bin_size,
                tile_size: options.heatmap_tile_size,
            },
        )?;
        eprintln!(
            "fovea-pack: prepared {} heatmap tiles in {:.2}s",
            heatmap.tiles.len(),
            start.elapsed().as_secs_f64()
        );
        Some(Arc::new(heatmap))
    } else {
        None
    };

    Ok(SlideSources {
        reader,
        slide_manifest,
        slide_manifest_json,
        image_format: options.image_format,
        tile_cache: Arc::new(Mutex::new(TileCache::new(
            options.tile_cache_mb.saturating_mul(1024 * 1024),
        ))),
        cells,
        heatmap,
    })
}

pub async fn serve_sources(options: ServeOptions) -> Result<()> {
    let state = prepare_sources(options.source_options()).await?;
    let app = Router::new()
        .fallback(get(handle_get).options(handle_options))
        .with_state(state);
    let address = SocketAddr::from((options.host, options.port));
    let viewer_url = viewer_url(
        address,
        options.cells_protobuf_path.is_some(),
        options.heatmap,
    );

    eprintln!("fovea-pack serve: listening on http://{address}");
    eprintln!("fovea-pack serve: open {viewer_url}");

    let listener = tokio::net::TcpListener::bind(address)
        .await
        .with_context(|| format!("failed to bind {address}"))?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn handle_options() -> Response {
    empty_response(StatusCode::NO_CONTENT)
}

async fn handle_get(State(state): State<SlideSources>, uri: Uri) -> Response {
    match route_request(&state, uri.path()).await {
        Ok(response) => response,
        Err(error) => {
            eprintln!("fovea-pack serve: request failed: {error:#}");
            text_response(StatusCode::INTERNAL_SERVER_ERROR, "internal server error")
        }
    }
}

/// Route one rendering-data request against a prepared [`SlideSources`]. `path`
/// is the request path relative to the slide root, e.g. `/slide/manifest.json`,
/// `/slide/images/level_0/0_0.jpg`, `/cells/manifest.json`, `/cells/chunks/0_0.fovc`,
/// `/heatmap/manifest.json`, or `/heatmap/tiles/0/0_0.fovh`. Returns a ready
/// axum [`Response`]; an embedding host can forward it verbatim.
pub async fn route_request(state: &SlideSources, path: &str) -> Result<Response> {
    match path {
        "/slide/manifest.json" => Ok(text_response(
            StatusCode::OK,
            state.slide_manifest_json.as_str(),
        )),
        "/cells/manifest.json" => {
            let Some(cells) = &state.cells else {
                return Ok(text_response(StatusCode::NOT_FOUND, "cells not loaded"));
            };
            Ok(text_response(StatusCode::OK, &cells.manifest_json))
        }
        "/heatmap/manifest.json" => {
            let Some(heatmap) = &state.heatmap else {
                return Ok(text_response(StatusCode::NOT_FOUND, "heatmap not loaded"));
            };
            Ok(text_response(StatusCode::OK, &heatmap.manifest_json))
        }
        _ if path.starts_with("/slide/images/") => serve_slide_tile(state, path).await,
        _ if path.starts_with("/cells/chunks/") => serve_cell_chunk(state, path),
        _ if path.starts_with("/heatmap/tiles/") => serve_heatmap_tile(state, path),
        _ => Ok(text_response(StatusCode::NOT_FOUND, "not found")),
    }
}

async fn serve_slide_tile(state: &SlideSources, path: &str) -> Result<Response> {
    let Some((level, x, y)) = parse_slide_tile_path(path, state.image_format) else {
        return Ok(text_response(StatusCode::NOT_FOUND, "tile not found"));
    };
    let key = TileKey { level, x, y };
    let start = Instant::now();

    if let Some(bytes) = state.tile_cache.lock().expect("tile cache lock").get(key) {
        log_request("slide tile hit", path, bytes.len(), start);
        return Ok(bytes_response(
            StatusCode::OK,
            state.image_format.content_type(),
            bytes,
        ));
    }

    let Some(request) = slide_tile_request(&state.slide_manifest, level, x, y) else {
        return Ok(text_response(StatusCode::NOT_FOUND, "tile not found"));
    };
    let reader = Arc::clone(&state.reader);
    let image_format = state.image_format;
    let bytes = tokio::task::spawn_blocking(move || {
        encode_slide_tile(reader.as_ref(), &request, image_format)
    })
    .await
    .context("tile worker failed")??;

    state
        .tile_cache
        .lock()
        .expect("tile cache lock")
        .insert(key, bytes.clone());
    log_request("slide tile miss", path, bytes.len(), start);
    Ok(bytes_response(
        StatusCode::OK,
        state.image_format.content_type(),
        bytes,
    ))
}

fn serve_cell_chunk(state: &SlideSources, path: &str) -> Result<Response> {
    let Some(cells) = &state.cells else {
        return Ok(text_response(StatusCode::NOT_FOUND, "cells not loaded"));
    };
    let Some((x, y)) = parse_xy_file(
        path.strip_prefix("/cells/chunks/").unwrap_or_default(),
        "fovc",
    ) else {
        return Ok(text_response(StatusCode::NOT_FOUND, "chunk not found"));
    };
    let start = Instant::now();
    let Some(bytes) = cells.chunks.get(&(x, y)).cloned() else {
        return Ok(text_response(StatusCode::NOT_FOUND, "chunk not found"));
    };

    log_request("cell chunk", path, bytes.len(), start);
    Ok(bytes_response(
        StatusCode::OK,
        "application/octet-stream",
        bytes,
    ))
}

fn serve_heatmap_tile(state: &SlideSources, path: &str) -> Result<Response> {
    let Some(heatmap) = &state.heatmap else {
        return Ok(text_response(StatusCode::NOT_FOUND, "heatmap not loaded"));
    };
    let Some((level, x, y)) = parse_heatmap_tile_path(path) else {
        return Ok(text_response(
            StatusCode::NOT_FOUND,
            "heatmap tile not found",
        ));
    };
    let start = Instant::now();
    let Some(bytes) = heatmap.tiles.get(&(level, x, y)).cloned() else {
        return Ok(text_response(
            StatusCode::NOT_FOUND,
            "heatmap tile not found",
        ));
    };

    log_request("heatmap tile", path, bytes.len(), start);
    Ok(bytes_response(
        StatusCode::OK,
        "application/octet-stream",
        bytes,
    ))
}

fn parse_slide_tile_path(path: &str, image_format: ImageFormat) -> Option<(u32, u32, u32)> {
    let rest = path.strip_prefix("/slide/images/")?;
    let mut parts = rest.split('/');
    let level = parts.next()?.strip_prefix("level_")?.parse().ok()?;
    let file = parts.next()?;

    if parts.next().is_some() {
        return None;
    }

    let (x, y) = parse_xy_file(file, image_format.extension())?;
    Some((level, x, y))
}

fn parse_heatmap_tile_path(path: &str) -> Option<(u32, u32, u32)> {
    let rest = path.strip_prefix("/heatmap/tiles/")?;
    let mut parts = rest.split('/');
    let level = parts.next()?.parse().ok()?;
    let file = parts.next()?;

    if parts.next().is_some() {
        return None;
    }

    let (x, y) = parse_xy_file(file, "fovh")?;
    Some((level, x, y))
}

fn parse_xy_file(file: &str, extension: &str) -> Option<(u32, u32)> {
    let stem = file.strip_suffix(&format!(".{extension}"))?;
    let (x, y) = stem.split_once('_')?;
    Some((x.parse().ok()?, y.parse().ok()?))
}

fn log_request(kind: &str, path: &str, bytes: usize, start: Instant) {
    eprintln!(
        "fovea-pack serve: {kind} {path} {} bytes {:.2} ms",
        bytes,
        start.elapsed().as_secs_f64() * 1000.0
    );
}

fn viewer_url(address: SocketAddr, has_cells: bool, has_heatmap: bool) -> String {
    let base = format!("http://{address}");
    let mut url = format!("http://127.0.0.1:5173/?slide={base}/slide");

    if has_cells {
        url.push_str(&format!("&cells={base}/cells"));
    }

    if has_heatmap {
        url.push_str(&format!("&heatmap={base}/heatmap"));
    }

    url
}

fn bytes_response(status: StatusCode, content_type: &'static str, bytes: Vec<u8>) -> Response {
    let mut response = (status, bytes).into_response();
    add_common_headers(response.headers_mut(), content_type);
    response
}

fn text_response(status: StatusCode, text: &str) -> Response {
    let mut response = (status, text.to_string()).into_response();
    add_common_headers(response.headers_mut(), "application/json; charset=utf-8");
    response
}

fn empty_response(status: StatusCode) -> Response {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = status;
    add_common_headers(response.headers_mut(), "text/plain; charset=utf-8");
    response
}

fn add_common_headers(headers: &mut header::HeaderMap, content_type: &'static str) {
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*".parse().unwrap());
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        "GET, OPTIONS".parse().unwrap(),
    );
    headers.insert(header::CONTENT_TYPE, content_type.parse().unwrap());
    headers.insert(header::CACHE_CONTROL, "no-cache".parse().unwrap());
}

trait ImageFormatContentType {
    fn content_type(self) -> &'static str;
}

impl ImageFormatContentType for ImageFormat {
    fn content_type(self) -> &'static str {
        match self {
            ImageFormat::Webp => "image/webp",
            ImageFormat::Jpeg => "image/jpeg",
            ImageFormat::Png => "image/png",
        }
    }
}

#[allow(dead_code)]
async fn _method_guard(request: Request<Body>) -> Response {
    if request.method() == Method::OPTIONS {
        empty_response(StatusCode::NO_CONTENT)
    } else {
        text_response(StatusCode::METHOD_NOT_ALLOWED, "method not allowed")
    }
}
