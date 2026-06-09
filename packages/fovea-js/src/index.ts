import initWasm, {
  FoveaViewer as WasmFoveaViewer,
  type FrameStats
} from "../pkg/fovea_viewer.js";

export type BenchmarkPointCount = 10_000 | 100_000 | 500_000 | 1_000_000;

export interface FoveaViewerOptions {
  canvas: HTMLCanvasElement;
  pointCount?: BenchmarkPointCount;
  onStats?: (stats: FrameStats, rolling: RollingFrameStats) => void;
}

export interface RollingFrameStats {
  frameTimeP50: number;
  frameTimeP95: number;
  frameTimeP99: number;
  fps: number;
}

export class FoveaViewer {
  private animationFrame = 0;
  private destroyed = false;
  private lastPointer: PointerEvent | null = null;
  private readonly frameTimes: number[] = [];
  private readonly resizeObserver: ResizeObserver;

  private constructor(
    private readonly wasm: WasmFoveaViewer,
    private readonly canvas: HTMLCanvasElement,
    private readonly onStats?: (stats: FrameStats, rolling: RollingFrameStats) => void
  ) {
    this.resizeObserver = new ResizeObserver(() => this.resize());
    this.resizeObserver.observe(canvas);
    this.bindInput();
    this.resize();
  }

  static async create(options: FoveaViewerOptions): Promise<FoveaViewer> {
    await initWasm();

    const wasm = await WasmFoveaViewer.create(options.canvas);
    const viewer = new FoveaViewer(wasm, options.canvas, options.onStats);

    if (options.pointCount) {
      viewer.setPointCount(options.pointCount);
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

      const stats = this.wasm.render();
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
    this.resizeObserver.disconnect();
  }

  setPointCount(count: BenchmarkPointCount): void {
    this.wasm.setPointCount(count);
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
    });

    this.canvas.addEventListener("pointermove", (event) => {
      if (!this.lastPointer) {
        return;
      }

      const dpr = window.devicePixelRatio || 1;
      this.wasm.panByScreenDelta(
        (event.clientX - this.lastPointer.clientX) * dpr,
        (event.clientY - this.lastPointer.clientY) * dpr
      );
      this.lastPointer = event;
    });

    this.canvas.addEventListener("pointerup", () => {
      this.lastPointer = null;
    });

    this.canvas.addEventListener("pointercancel", () => {
      this.lastPointer = null;
    });

    this.canvas.addEventListener(
      "wheel",
      (event) => {
        event.preventDefault();
        const rect = this.canvas.getBoundingClientRect();
        const dpr = window.devicePixelRatio || 1;
        this.wasm.zoomAt(
          (event.clientX - rect.left) * dpr,
          (event.clientY - rect.top) * dpr,
          event.deltaY
        );
      },
      { passive: false }
    );
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
}

export type { FrameStats };

