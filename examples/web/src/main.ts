import { FoveaViewer, type BenchmarkPointCount } from "@fovea/js";
import "./styles.css";

const canvas = document.querySelector<HTMLCanvasElement>("#viewer");
const pointSelect = document.querySelector<HTMLSelectElement>("#point-count");
const resetButton = document.querySelector<HTMLButtonElement>("#reset-camera");

if (!canvas || !pointSelect || !resetButton) {
  throw new Error("Fovea example DOM is incomplete");
}

const viewerCanvas = canvas;
const pointCountSelect = pointSelect;
const resetCameraButton = resetButton;

const values = {
  fps: document.querySelector<HTMLElement>("#fps"),
  p50: document.querySelector<HTMLElement>("#p50"),
  p95: document.querySelector<HTMLElement>("#p95"),
  p99: document.querySelector<HTMLElement>("#p99"),
  frame: document.querySelector<HTMLElement>("#frame"),
  upload: document.querySelector<HTMLElement>("#upload"),
  draws: document.querySelector<HTMLElement>("#draws"),
  visible: document.querySelector<HTMLElement>("#visible"),
  gpu: document.querySelector<HTMLElement>("#gpu"),
  cpu: document.querySelector<HTMLElement>("#cpu")
};

function setText(key: keyof typeof values, text: string): void {
  const element = values[key];

  if (element) {
    element.textContent = text;
  }
}

function formatMs(value: number): string {
  return `${value.toFixed(2)} ms`;
}

function formatBytes(value: number): string {
  return `${(value / (1024 * 1024)).toFixed(2)} MB`;
}

async function main(): Promise<void> {
  const viewer = await FoveaViewer.create({
    canvas: viewerCanvas,
    pointCount: Number(pointCountSelect.value) as BenchmarkPointCount,
    onStats: (stats, rolling) => {
      setText("fps", rolling.fps.toFixed(1));
      setText("p50", formatMs(rolling.frameTimeP50));
      setText("p95", formatMs(rolling.frameTimeP95));
      setText("p99", formatMs(rolling.frameTimeP99));
      setText("frame", formatMs(stats.frameTimeMs));
      setText("upload", formatMs(stats.uploadTimeMs));
      setText("draws", String(stats.drawCallCount));
      setText("visible", stats.visibleObjectCount.toLocaleString());
      setText("gpu", formatBytes(stats.gpuBufferMemoryBytes));
      setText("cpu", formatBytes(stats.cpuMemoryBytes));
    }
  });

  pointCountSelect.addEventListener("change", () => {
    viewer.setPointCount(Number(pointCountSelect.value) as BenchmarkPointCount);
  });

  resetCameraButton.addEventListener("click", () => viewer.resetCamera());

  viewer.start();
}

void main();
