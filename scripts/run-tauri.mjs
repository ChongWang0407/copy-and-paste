import { spawn } from "node:child_process";

const args = process.argv.slice(2);
const isDev = args[0] === "dev";
const hasExplicitConfig = args.some(
  (arg) => arg === "--config" || arg.startsWith("--config=")
);

if (isDev && !hasExplicitConfig) {
  args.splice(1, 0, "--config", "src-tauri/tauri.dev.conf.json");
}

const command = process.platform === "win32" ? "tauri.cmd" : "tauri";
const child = spawn(command, args, {
  stdio: "inherit",
  shell: process.platform === "win32"
});

child.on("error", (error) => {
  console.error(`[run-tauri] Failed to start Tauri: ${error.message}`);
  process.exitCode = 1;
});

child.on("exit", (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
    return;
  }
  process.exitCode = code ?? 1;
});
