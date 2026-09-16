import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";

const root = resolve(dirname(new URL(import.meta.url).pathname), "..");
const wasmTarget = "wasm32-unknown-unknown";
const wasmBindgenVersion = "0.2.108";

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: root,
    stdio: "inherit",
    shell: process.platform === "win32",
    ...options
  });

  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

function commandVersion(command) {
  const result = spawnSync(command, ["--version"], {
    cwd: root,
    encoding: "utf8",
    shell: process.platform === "win32"
  });

  if (result.status !== 0) {
    return "";
  }

  return result.stdout.trim();
}

// No-op when the target is already installed.
run("rustup", ["target", "add", wasmTarget]);

if (!commandVersion("wasm-bindgen").includes(wasmBindgenVersion)) {
  run("cargo", [
    "install",
    "wasm-bindgen-cli",
    "--version",
    wasmBindgenVersion,
    "--locked",
    "--force"
  ]);
}

const outDir = resolve(root, "packages/fovea-js/pkg");

run("cargo", ["build", "--release", "--target", wasmTarget, "-p", "fovea-viewer"]);

const wasmPath = resolve(root, "target/wasm32-unknown-unknown/release/fovea_viewer.wasm");

// wasm-bindgen creates outDir and reports a missing input file itself.
run("wasm-bindgen", [
  "--target",
  "web",
  "--out-dir",
  outDir,
  "--out-name",
  "fovea_viewer",
  wasmPath
]);
