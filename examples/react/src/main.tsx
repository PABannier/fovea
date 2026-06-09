import { FoveaViewer, type PerformanceStats } from "@fovea/viewer";
import React, { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import "./styles.css";

function App(): React.ReactElement {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const viewerRef = useRef<FoveaViewer | null>(null);
  const [stats, setStats] = useState<PerformanceStats | null>(null);
  const [status, setStatus] = useState("Synthetic");

  useEffect(() => {
    let cancelled = false;

    async function startViewer(): Promise<void> {
      if (!canvasRef.current) {
        return;
      }

      const params = new URLSearchParams(window.location.search);
      const viewer = await FoveaViewer.create({
        canvas: canvasRef.current,
        slideUrl: params.get("slide") ?? undefined,
        cellsUrl: params.get("cells") ?? undefined,
        heatmapUrl: params.get("heatmap") ?? undefined,
        onStats: () => {
          if (!cancelled) {
            setStats(viewer.getPerformanceStats());
          }
        }
      });

      if (cancelled) {
        viewer.destroy();
        return;
      }

      viewerRef.current = viewer;
      viewer.start();
      setStatus(
        [params.get("slide") && "Slide", params.get("cells") && "Cells", params.get("heatmap") && "Heatmap"]
          .filter(Boolean)
          .join(" + ") || "Synthetic"
      );
    }

    void startViewer();

    return () => {
      cancelled = true;
      viewerRef.current?.destroy();
      viewerRef.current = null;
    };
  }, []);

  return (
    <main>
      <canvas ref={canvasRef} />
      <section className="panel">
        <button type="button" onClick={() => viewerRef.current?.resetCamera()}>
          Reset
        </button>
        <strong>{status}</strong>
        <span>{stats ? `${stats.fps.toFixed(1)} FPS` : "Starting"}</span>
        <span>{stats ? `${stats.drawCalls} draws` : "0 draws"}</span>
        <span>{stats ? `${stats.visibleObjects.toLocaleString()} visible` : "0 visible"}</span>
      </section>
    </main>
  );
}

const root = document.querySelector("#root");

if (!root) {
  throw new Error("React root is missing");
}

createRoot(root).render(<App />);
