import initWasm, {
  FoveaViewer as WasmFoveaViewer,
  type FrameStats
} from "../pkg/fovea_viewer.js";

export type BenchmarkPointCount = 10_000 | 100_000 | 500_000 | 1_000_000;

export interface FoveaViewerOptions {
  canvas: HTMLCanvasElement;
  pointCount?: BenchmarkPointCount;
  bundleUrl?: string;
  overlayUrl?: string;
  heatmapUrl?: string;
  tileRequestBatchSize?: number;
  overlayRequestBatchSize?: number;
  heatmapRequestBatchSize?: number;
  maxConcurrentTileRequests?: number;
  maxConcurrentOverlayRequests?: number;
  maxConcurrentHeatmapRequests?: number;
  onStats?: (stats: FrameStats, rolling: RollingFrameStats) => void;
}

export interface RollingFrameStats {
  frameTimeP50: number;
  frameTimeP95: number;
  frameTimeP99: number;
  fps: number;
}

export interface CellEvent {
  cellId: number | null;
  classId: number | null;
  slideX: number | null;
  slideY: number | null;
}

export interface SelectionChangeEvent {
  count: number;
}

export interface ViewportChangeEvent {
  centerX: number;
  centerY: number;
  zoom: number;
}

export interface FoveaViewerEvents {
  "cell-hover": CellEvent;
  "cell-click": CellEvent;
  "selection-change": SelectionChangeEvent;
  "viewport-change": ViewportChangeEvent;
}

type EventCallback<T> = (event: T) => void;

type RawViewerEvent =
  | ({ type: "cell-hover" } & CellEvent)
  | ({ type: "cell-click" } & CellEvent)
  | ({ type: "selection-change" } & SelectionChangeEvent)
  | ({ type: "viewport-changed" } & ViewportChangeEvent);

interface TileRequest {
  level: number;
  x: number;
  y: number;
  width: number;
  height: number;
  path: string;
  priority: number;
}

interface OverlayChunkRequest {
  x: number;
  y: number;
  path: string;
  cellCount: number;
  byteSize: number;
  priority: number;
}

interface HeatmapTileRequest {
  level: number;
  x: number;
  y: number;
  width: number;
  height: number;
  path: string;
  byteSize: number;
  priority: number;
}

export class FoveaViewer {
  private animationFrame = 0;
  private destroyed = false;
  private bundleVersion = 0;
  private overlayVersion = 0;
  private heatmapVersion = 0;
  private tileBaseUrl: string | null = null;
  private overlayBaseUrl: string | null = null;
  private heatmapBaseUrl: string | null = null;
  private lastPointer: PointerEvent | null = null;
  private pointerDown: { clientX: number; clientY: number } | null = null;
  private readonly tileRequestBatchSize: number;
  private readonly overlayRequestBatchSize: number;
  private readonly heatmapRequestBatchSize: number;
  private readonly maxConcurrentTileRequests: number;
  private readonly maxConcurrentOverlayRequests: number;
  private readonly maxConcurrentHeatmapRequests: number;
  private readonly inflightTiles = new Map<string, AbortController>();
  private readonly inflightOverlayChunks = new Map<string, AbortController>();
  private readonly inflightHeatmapTiles = new Map<string, AbortController>();
  private readonly eventListeners = new Map<keyof FoveaViewerEvents, Set<EventCallback<any>>>();
  private readonly frameTimes: number[] = [];
  private readonly resizeObserver: ResizeObserver;

  private constructor(
    private readonly wasm: WasmFoveaViewer,
    private readonly canvas: HTMLCanvasElement,
    options: FoveaViewerOptions
  ) {
    this.tileRequestBatchSize = options.tileRequestBatchSize ?? 96;
    this.overlayRequestBatchSize = options.overlayRequestBatchSize ?? 64;
    this.heatmapRequestBatchSize = options.heatmapRequestBatchSize ?? 64;
    this.maxConcurrentTileRequests = options.maxConcurrentTileRequests ?? 8;
    this.maxConcurrentOverlayRequests = options.maxConcurrentOverlayRequests ?? 6;
    this.maxConcurrentHeatmapRequests = options.maxConcurrentHeatmapRequests ?? 6;
    this.resizeObserver = new ResizeObserver(() => this.resize());
    this.resizeObserver.observe(canvas);
    this.bindInput();
    this.resize();
    this.onStats = options.onStats;
  }

  private readonly onStats?: (stats: FrameStats, rolling: RollingFrameStats) => void;

  static async create(options: FoveaViewerOptions): Promise<FoveaViewer> {
    await initWasm();

    const wasm = await WasmFoveaViewer.create(options.canvas);
    const viewer = new FoveaViewer(wasm, options.canvas, options);

    if (options.pointCount) {
      viewer.setPointCount(options.pointCount);
    }

    if (options.bundleUrl) {
      await viewer.loadBundle(options.bundleUrl);
    }

    if (options.overlayUrl) {
      await viewer.loadOverlay(options.overlayUrl);
    }

    if (options.heatmapUrl) {
      await viewer.loadHeatmap(options.heatmapUrl);
    }

    return viewer;
  }

  start(): void {
    if (this.animationFrame !== 0) {
      return;
    }

    const tick = () => {
      if (this.destroyed) {
        return;
      }

      this.pumpTileRequests();
      this.pumpHeatmapRequests();
      this.pumpOverlayRequests();
      const stats = this.wasm.render();
      this.dispatchDrainedEvents();
      this.observeFrame(stats);
      this.animationFrame = requestAnimationFrame(tick);
    };

    this.animationFrame = requestAnimationFrame(tick);
  }

  stop(): void {
    if (this.animationFrame !== 0) {
      cancelAnimationFrame(this.animationFrame);
      this.animationFrame = 0;
    }
  }

  destroy(): void {
    this.destroyed = true;
    this.stop();
    this.abortInflightTiles();
    this.abortInflightOverlayChunks();
    this.abortInflightHeatmapTiles();
    this.resizeObserver.disconnect();
  }

  async loadBundle(bundleUrl: string): Promise<void> {
    const version = ++this.bundleVersion;
    this.abortInflightTiles();

    const manifestUrl = manifestUrlForBundle(bundleUrl);
    const response = await fetch(manifestUrl, { cache: "no-cache" });

    if (!response.ok) {
      throw new Error(`Failed to fetch manifest: ${response.status} ${response.statusText}`);
    }

    const manifestJson = await response.text();

    if (version !== this.bundleVersion || this.destroyed) {
      return;
    }

    this.tileBaseUrl = new URL(".", manifestUrl).toString();
    this.wasm.loadManifest(manifestJson);
    this.frameTimes.length = 0;
  }

  async loadOverlay(overlayUrl: string): Promise<void> {
    const version = ++this.overlayVersion;
    this.abortInflightOverlayChunks();

    const manifestUrl = manifestUrlForBundle(overlayUrl);
    const response = await fetch(manifestUrl, { cache: "no-cache" });

    if (!response.ok) {
      throw new Error(`Failed to fetch overlay manifest: ${response.status} ${response.statusText}`);
    }

    const manifestJson = await response.text();

    if (version !== this.overlayVersion || this.destroyed) {
      return;
    }

    this.overlayBaseUrl = new URL(".", manifestUrl).toString();
    this.wasm.loadOverlayManifest(manifestJson);
    this.frameTimes.length = 0;
  }

  async loadHeatmap(heatmapUrl: string): Promise<void> {
    const version = ++this.heatmapVersion;
    this.abortInflightHeatmapTiles();

    const manifestUrl = manifestUrlForBundle(heatmapUrl);
    const response = await fetch(manifestUrl, { cache: "no-cache" });

    if (!response.ok) {
      throw new Error(`Failed to fetch heatmap manifest: ${response.status} ${response.statusText}`);
    }

    const manifestJson = await response.text();

    if (version !== this.heatmapVersion || this.destroyed) {
      return;
    }

    this.heatmapBaseUrl = new URL(".", manifestUrl).toString();
    this.wasm.loadHeatmapManifest(manifestJson);
    this.frameTimes.length = 0;
  }

  setPointCount(count: BenchmarkPointCount): void {
    this.wasm.setPointCount(count);
  }

  setLayerVisibility(layerId: "cells" | "heatmap" | string, visible: boolean): void {
    if (layerId === "cells") {
      this.wasm.setOverlayVisibility(visible);
    } else {
      this.wasm.setHeatmapVisibility(visible);
    }
  }

  setLayerOpacity(layerId: "cells" | "heatmap" | string, opacity: number): void {
    if (layerId === "cells") {
      this.wasm.setOverlayOpacity(opacity);
    } else {
      this.wasm.setHeatmapOpacity(opacity);
    }
  }

  setHeatmapRange(_layerId: "heatmap" | string, range: { min: number; max: number }): void {
    this.wasm.setHeatmapRange(range.min, range.max);
  }

  setHeatmapColormap(_layerId: "heatmap" | string, colormap: "magma" | "viridis" | "gray"): void {
    this.wasm.setHeatmapColormap(colormap);
  }

  setOverlayPointSize(sizePx: number): void {
    this.wasm.setOverlayPointSize(sizePx);
  }

  setOverlayOutlineWidth(widthPx: number): void {
    this.wasm.setOverlayOutlineWidth(widthPx);
  }

  on<K extends keyof FoveaViewerEvents>(
    eventName: K,
    callback: EventCallback<FoveaViewerEvents[K]>
  ): () => void {
    let listeners = this.eventListeners.get(eventName);

    if (!listeners) {
      listeners = new Set();
      this.eventListeners.set(eventName, listeners);
    }

    listeners.add(callback as EventCallback<any>);
    return () => listeners?.delete(callback as EventCallback<any>);
  }

  resetCamera(): void {
    this.wasm.resetCamera();
  }

  private resize(): void {
    const rect = this.canvas.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    const width = Math.max(1, Math.round(rect.width * dpr));
    const height = Math.max(1, Math.round(rect.height * dpr));

    if (this.canvas.width !== width || this.canvas.height !== height) {
      this.canvas.width = width;
      this.canvas.height = height;
    }

    this.wasm.resize(width, height, dpr);
  }

  private bindInput(): void {
    this.canvas.addEventListener("pointerdown", (event) => {
      this.canvas.setPointerCapture(event.pointerId);
      this.lastPointer = event;
      this.pointerDown = {
        clientX: event.clientX,
        clientY: event.clientY
      };
    });

    this.canvas.addEventListener("pointermove", (event) => {
      if (!this.lastPointer) {
        this.hoverAtEvent(event);
        return;
      }

      const scale = this.canvasScale();
      this.wasm.panByScreenDelta(
        (event.clientX - this.lastPointer.clientX) * scale.x,
        (event.clientY - this.lastPointer.clientY) * scale.y
      );
      this.lastPointer = event;
    });

    this.canvas.addEventListener("pointerup", (event) => {
      if (this.pointerDown) {
        const moved = Math.hypot(
          event.clientX - this.pointerDown.clientX,
          event.clientY - this.pointerDown.clientY
        );

        if (moved <= 4) {
          this.clickAtEvent(event);
        } else {
          this.hoverAtEvent(event);
        }
      }

      this.lastPointer = null;
      this.pointerDown = null;
    });

    this.canvas.addEventListener("pointercancel", () => {
      this.lastPointer = null;
      this.pointerDown = null;
    });

    this.canvas.addEventListener(
      "wheel",
      (event) => {
        event.preventDefault();
        const point = this.eventCanvasPoint(event);
        this.wasm.zoomAt(
          point.x,
          point.y,
          event.deltaY
        );
      },
      { passive: false }
    );
  }

  private hoverAtEvent(event: PointerEvent): void {
    const point = this.eventCanvasPoint(event);
    this.wasm.hoverAt(point.x, point.y);
  }

  private clickAtEvent(event: PointerEvent): void {
    const point = this.eventCanvasPoint(event);
    this.wasm.clickAt(point.x, point.y);
  }

  private eventCanvasPoint(event: Pick<MouseEvent, "clientX" | "clientY">): { x: number; y: number } {
    const rect = this.canvas.getBoundingClientRect();
    const scale = this.canvasScale(rect);

    return {
      x: (event.clientX - rect.left) * scale.x,
      y: (event.clientY - rect.top) * scale.y
    };
  }

  private canvasScale(rect = this.canvas.getBoundingClientRect()): { x: number; y: number } {
    return {
      x: this.canvas.width / Math.max(1, rect.width),
      y: this.canvas.height / Math.max(1, rect.height)
    };
  }

  private pumpTileRequests(): void {
    if (!this.tileBaseUrl || this.destroyed) {
      return;
    }

    const requests = this.parseVisibleTileRequests();
    const wanted = new Set(requests.map((request) => tileKey(request)));

    for (const [key, controller] of this.inflightTiles) {
      if (!wanted.has(key)) {
        controller.abort();
        this.inflightTiles.delete(key);
      }
    }

    for (const request of requests) {
      if (this.inflightTiles.size >= this.maxConcurrentTileRequests) {
        break;
      }

      const key = tileKey(request);

      if (this.inflightTiles.has(key)) {
        continue;
      }

      const controller = new AbortController();
      const version = this.bundleVersion;
      this.inflightTiles.set(key, controller);
      void this.loadTile(request, controller, version)
        .catch((error: unknown) => {
          if (!controller.signal.aborted) {
            console.error(error);
          }
        })
        .finally(() => {
          if (this.inflightTiles.get(key) === controller) {
            this.inflightTiles.delete(key);
          }
        });
    }
  }

  private parseVisibleTileRequests(): TileRequest[] {
    const raw = this.wasm.visibleTileRequests(this.tileRequestBatchSize);

    try {
      const requests = JSON.parse(raw) as TileRequest[];
      return requests.sort((a, b) => a.priority - b.priority);
    } catch {
      return [];
    }
  }

  private pumpOverlayRequests(): void {
    if (!this.overlayBaseUrl || this.destroyed) {
      return;
    }

    const requests = this.parseVisibleOverlayChunkRequests();
    const wanted = new Set(requests.map((request) => overlayChunkKey(request)));

    for (const [key, controller] of this.inflightOverlayChunks) {
      if (!wanted.has(key)) {
        controller.abort();
        this.inflightOverlayChunks.delete(key);
      }
    }

    for (const request of requests) {
      if (this.inflightOverlayChunks.size >= this.maxConcurrentOverlayRequests) {
        break;
      }

      const key = overlayChunkKey(request);

      if (this.inflightOverlayChunks.has(key)) {
        continue;
      }

      const controller = new AbortController();
      const version = this.overlayVersion;
      this.inflightOverlayChunks.set(key, controller);
      void this.loadOverlayChunk(request, controller, version)
        .catch((error: unknown) => {
          if (!controller.signal.aborted) {
            console.error(error);
          }
        })
        .finally(() => {
          if (this.inflightOverlayChunks.get(key) === controller) {
            this.inflightOverlayChunks.delete(key);
          }
        });
    }
  }

  private pumpHeatmapRequests(): void {
    if (!this.heatmapBaseUrl || this.destroyed) {
      return;
    }

    const requests = this.parseVisibleHeatmapTileRequests();
    const wanted = new Set(requests.map((request) => heatmapTileKey(request)));

    for (const [key, controller] of this.inflightHeatmapTiles) {
      if (!wanted.has(key)) {
        controller.abort();
        this.inflightHeatmapTiles.delete(key);
      }
    }

    for (const request of requests) {
      if (this.inflightHeatmapTiles.size >= this.maxConcurrentHeatmapRequests) {
        break;
      }

      const key = heatmapTileKey(request);

      if (this.inflightHeatmapTiles.has(key)) {
        continue;
      }

      const controller = new AbortController();
      const version = this.heatmapVersion;
      this.inflightHeatmapTiles.set(key, controller);
      void this.loadHeatmapTile(request, controller, version)
        .catch((error: unknown) => {
          if (!controller.signal.aborted) {
            console.error(error);
          }
        })
        .finally(() => {
          if (this.inflightHeatmapTiles.get(key) === controller) {
            this.inflightHeatmapTiles.delete(key);
          }
        });
    }
  }

  private parseVisibleHeatmapTileRequests(): HeatmapTileRequest[] {
    const raw = this.wasm.visibleHeatmapTileRequests(this.heatmapRequestBatchSize);

    try {
      const requests = JSON.parse(raw) as HeatmapTileRequest[];
      return requests.sort((a, b) => a.priority - b.priority);
    } catch {
      return [];
    }
  }

  private async loadHeatmapTile(
    request: HeatmapTileRequest,
    controller: AbortController,
    version: number
  ): Promise<void> {
    const heatmapBaseUrl = this.heatmapBaseUrl;

    if (!heatmapBaseUrl) {
      return;
    }

    const tileUrl = new URL(request.path, heatmapBaseUrl);
    const response = await fetch(tileUrl, {
      cache: "force-cache",
      signal: controller.signal
    });

    if (!response.ok) {
      throw new Error(`Failed to fetch heatmap tile ${request.path}: ${response.status}`);
    }

    const bytes = new Uint8Array(await response.arrayBuffer());

    if (controller.signal.aborted || version !== this.heatmapVersion || this.destroyed) {
      return;
    }

    this.wasm.uploadHeatmapTileBytes(
      request.level,
      request.x,
      request.y,
      request.width,
      request.height,
      bytes
    );
  }

  private parseVisibleOverlayChunkRequests(): OverlayChunkRequest[] {
    const raw = this.wasm.visibleOverlayChunkRequests(this.overlayRequestBatchSize);

    try {
      const requests = JSON.parse(raw) as OverlayChunkRequest[];
      return requests.sort((a, b) => a.priority - b.priority);
    } catch {
      return [];
    }
  }

  private async loadOverlayChunk(
    request: OverlayChunkRequest,
    controller: AbortController,
    version: number
  ): Promise<void> {
    const overlayBaseUrl = this.overlayBaseUrl;

    if (!overlayBaseUrl) {
      return;
    }

    const chunkUrl = new URL(request.path, overlayBaseUrl);
    const response = await fetch(chunkUrl, {
      cache: "force-cache",
      signal: controller.signal
    });

    if (!response.ok) {
      throw new Error(`Failed to fetch overlay chunk ${request.path}: ${response.status}`);
    }

    const bytes = new Uint8Array(await response.arrayBuffer());

    if (controller.signal.aborted || version !== this.overlayVersion || this.destroyed) {
      return;
    }

    this.wasm.uploadOverlayChunkBytes(request.x, request.y, bytes);
  }

  private async loadTile(
    request: TileRequest,
    controller: AbortController,
    version: number
  ): Promise<void> {
    const tileBaseUrl = this.tileBaseUrl;

    if (!tileBaseUrl) {
      return;
    }

    const tileUrl = new URL(request.path, tileBaseUrl);
    const response = await fetch(tileUrl, {
      cache: "force-cache",
      signal: controller.signal
    });

    if (!response.ok) {
      throw new Error(`Failed to fetch tile ${request.path}: ${response.status}`);
    }

    const blob = await response.blob();

    if (controller.signal.aborted || version !== this.bundleVersion || this.destroyed) {
      return;
    }

    const bitmap = await createImageBitmap(blob);

    try {
      const rgba = decodeBitmapRgba(bitmap, request.width, request.height);

      if (controller.signal.aborted || version !== this.bundleVersion || this.destroyed) {
        return;
      }

      this.wasm.uploadTileRgba(
        request.level,
        request.x,
        request.y,
        rgba.width,
        rgba.height,
        rgba.data
      );
    } finally {
      bitmap.close();
    }
  }

  private observeFrame(stats: FrameStats): void {
    this.frameTimes.push(stats.frameTimeMs);

    if (this.frameTimes.length > 180) {
      this.frameTimes.shift();
    }

    if (!this.onStats) {
      return;
    }

    const sorted = [...this.frameTimes].sort((a, b) => a - b);
    const percentile = (p: number) =>
      sorted[Math.min(sorted.length - 1, Math.floor((sorted.length - 1) * p))] ?? 0;
    const p50 = percentile(0.5);
    const rolling = {
      frameTimeP50: p50,
      frameTimeP95: percentile(0.95),
      frameTimeP99: percentile(0.99),
      fps: p50 > 0 ? 1000 / p50 : 0
    };

    this.onStats(stats, rolling);
  }

  private dispatchDrainedEvents(): void {
    let events: RawViewerEvent[];

    try {
      events = JSON.parse(this.wasm.drainEvents()) as RawViewerEvent[];
    } catch {
      return;
    }

    for (const event of events) {
      if (event.type === "viewport-changed") {
        this.dispatchEvent("viewport-change", {
          centerX: event.centerX,
          centerY: event.centerY,
          zoom: event.zoom
        });
        continue;
      }

      this.dispatchEvent(event.type, event as FoveaViewerEvents[typeof event.type]);
    }
  }

  private dispatchEvent<K extends keyof FoveaViewerEvents>(
    eventName: K,
    event: FoveaViewerEvents[K]
  ): void {
    const listeners = this.eventListeners.get(eventName);

    if (!listeners) {
      return;
    }

    for (const listener of listeners) {
      listener(event);
    }
  }

  private abortInflightTiles(): void {
    for (const controller of this.inflightTiles.values()) {
      controller.abort();
    }

    this.inflightTiles.clear();
  }

  private abortInflightOverlayChunks(): void {
    for (const controller of this.inflightOverlayChunks.values()) {
      controller.abort();
    }

    this.inflightOverlayChunks.clear();
  }

  private abortInflightHeatmapTiles(): void {
    for (const controller of this.inflightHeatmapTiles.values()) {
      controller.abort();
    }

    this.inflightHeatmapTiles.clear();
  }
}

export type { FrameStats };

function manifestUrlForBundle(bundleUrl: string): URL {
  const url = new URL(bundleUrl, window.location.href);

  if (url.pathname.endsWith(".json")) {
    return url;
  }

  const pathname = url.pathname.endsWith("/") ? url.pathname : `${url.pathname}/`;
  return new URL(`${pathname}manifest.json${url.search}`, url);
}

function tileKey(request: TileRequest): string {
  return `${request.level}/${request.x}/${request.y}`;
}

function overlayChunkKey(request: OverlayChunkRequest): string {
  return `${request.x}/${request.y}`;
}

function heatmapTileKey(request: HeatmapTileRequest): string {
  return `${request.level}/${request.x}/${request.y}`;
}

function decodeBitmapRgba(
  bitmap: ImageBitmap,
  expectedWidth: number,
  expectedHeight: number
): { data: Uint8Array; width: number; height: number } {
  const width = bitmap.width || expectedWidth;
  const height = bitmap.height || expectedHeight;
  const canvas =
    typeof OffscreenCanvas !== "undefined"
      ? new OffscreenCanvas(width, height)
      : document.createElement("canvas");

  canvas.width = width;
  canvas.height = height;

  const context = canvas.getContext("2d", { willReadFrequently: true });

  if (!context) {
    throw new Error("2D canvas context unavailable for tile decode");
  }

  context.clearRect(0, 0, width, height);
  context.drawImage(bitmap, 0, 0, width, height);

  const imageData = context.getImageData(0, 0, width, height);
  return {
    data: new Uint8Array(
      imageData.data.buffer,
      imageData.data.byteOffset,
      imageData.data.byteLength
    ),
    width,
    height
  };
}
