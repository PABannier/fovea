import { FoveaViewer, type PerformanceStats } from "@fovea/viewer";
import "./styles.css";

declare global {
  interface Window {
    __foveaBenchmark?: BenchmarkResult;
  }
}

interface BenchmarkResult {
  status: "running" | "complete" | "failed";
  durationMs: number;
  samples: number;
  stats: PerformanceStats;
  error?: string;
}

const canvas = document.querySelector<HTMLCanvasElement>("#viewer");
const output = document.querySelector<HTMLPreElement>("#benchmark-output");

if (!canvas || !output) {
  throw new Error("Benchmark DOM is incomplete");
}

const params = new URLSearchParams(window.location.search);
const sampleMs = Number(params.get("durationMs") ?? "10000");
const viewer = await FoveaViewer.create({
  canvas,
  bundleUrl: params.get("bundle") ?? undefined,
  overlayUrl: params.get("overlay") ?? undefined,
  heatmapUrl: params.get("heatmap") ?? undefined
});

viewer.start();
setBenchmarkResult({
  status: "running",
  durationMs: 0,
  samples: 0,
  stats: viewer.getPerformanceStats()
});

try {
  await sleep(750);
  const startedAt = performance.now();
  const samples: PerformanceStats[] = [];

  while (performance.now() - startedAt < sampleMs) {
    const elapsed = performance.now() - startedAt;
    driveViewport(elapsed);

    if (elapsed > sampleMs * 0.35 && elapsed < sampleMs * 0.38) {
      viewer.setLayerVisibility("cells", false);
    } else if (elapsed > sampleMs * 0.42 && elapsed < sampleMs * 0.45) {
      viewer.setLayerVisibility("cells", true);
    } else if (elapsed > sampleMs * 0.65 && elapsed < sampleMs * 0.68) {
      viewer.setLayerVisibility("heatmap", false);
    } else if (elapsed > sampleMs * 0.72 && elapsed < sampleMs * 0.75) {
      viewer.setLayerVisibility("heatmap", true);
    }

    samples.push(viewer.getPerformanceStats());
    setBenchmarkResult({
      status: "running",
      durationMs: elapsed,
      samples: samples.length,
      stats: viewer.getPerformanceStats()
    });
    await nextFrame();
  }

  setBenchmarkResult({
    status: "complete",
    durationMs: performance.now() - startedAt,
    samples: samples.length,
    stats: summarize(samples)
  });
} catch (error) {
  setBenchmarkResult({
    status: "failed",
    durationMs: 0,
    samples: 0,
    stats: viewer.getPerformanceStats(),
    error: error instanceof Error ? error.message : String(error)
  });
}

function driveViewport(elapsedMs: number): void {
  const rect = canvas!.getBoundingClientRect();
  const centerX = rect.width * 0.5;
  const centerY = rect.height * 0.5;
  const phase = elapsedMs / 1000;
  const dx = Math.cos(phase * 1.7) * 8;
  const dy = Math.sin(phase * 1.3) * 6;

  viewer.panByScreenDelta(dx, dy);
  viewer.zoomAtCanvasPoint(centerX, centerY, Math.sin(phase) * 32);
}

function summarize(samples: PerformanceStats[]): PerformanceStats {
  if (samples.length === 0) {
    return viewer.getPerformanceStats();
  }

  const latest = samples[samples.length - 1];
  return {
    ...latest,
    fps: percentile(samples.map((sample) => sample.fps), 0.5),
    frameTimeP50Ms: percentile(samples.map((sample) => sample.frameTimeMs), 0.5),
    frameTimeP95Ms: percentile(samples.map((sample) => sample.frameTimeMs), 0.95),
    frameTimeP99Ms: percentile(samples.map((sample) => sample.frameTimeMs), 0.99),
    frameTimeMs: percentile(samples.map((sample) => sample.frameTimeMs), 0.5),
    uploadTimeMs: percentile(samples.map((sample) => sample.uploadTimeMs), 0.95),
    drawCalls: Math.max(...samples.map((sample) => sample.drawCalls)),
    visibleTiles: Math.max(...samples.map((sample) => sample.visibleTiles)),
    loadedTiles: Math.max(...samples.map((sample) => sample.loadedTiles)),
    visibleHeatmapTiles: Math.max(...samples.map((sample) => sample.visibleHeatmapTiles)),
    loadedHeatmapTiles: Math.max(...samples.map((sample) => sample.loadedHeatmapTiles)),
    visibleCellChunks: Math.max(...samples.map((sample) => sample.visibleCellChunks)),
    loadedCellChunks: Math.max(...samples.map((sample) => sample.loadedCellChunks)),
    visibleCells: Math.max(...samples.map((sample) => sample.visibleCells)),
    visibleObjects: Math.max(...samples.map((sample) => sample.visibleObjects)),
    gpuMemoryMb: Math.max(...samples.map((sample) => sample.gpuMemoryMb)),
    cpuMemoryMbEstimate: Math.max(...samples.map((sample) => sample.cpuMemoryMbEstimate)),
    gpuBufferMemoryBytes: Math.max(...samples.map((sample) => sample.gpuBufferMemoryBytes)),
    cpuMemoryBytes: Math.max(...samples.map((sample) => sample.cpuMemoryBytes))
  };
}

function percentile(values: number[], p: number): number {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.floor((sorted.length - 1) * p))] ?? 0;
}

function setBenchmarkResult(result: BenchmarkResult): void {
  window.__foveaBenchmark = result;
  output!.textContent = JSON.stringify(result, null, 2);
}

function nextFrame(): Promise<void> {
  return new Promise((resolve) => requestAnimationFrame(() => resolve()));
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
