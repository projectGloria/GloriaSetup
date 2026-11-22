#!/usr/bin/env node
"use strict";

const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const { packPath } = require("./index.cjs");

if (process.platform !== "win32") {
  console.error("gloria-setup: gloria-pack only runs on Windows (win32 x64).");
  process.exit(1);
}

if (!fs.existsSync(packPath)) {
  console.error(
    `gloria-setup: bundled binary not found at ${packPath}. Reinstall gloria-setup or run tools/stage-npm.ps1 in a source checkout.`
  );
  process.exit(1);
}

const result = spawnSync(packPath, process.argv.slice(2), {
  stdio: "inherit",
});

if (result.error) {
  console.error(`gloria-setup: failed to start gloria-pack: ${result.error.message}`);
  process.exit(1);
}

if (result.signal) {
  process.kill(process.pid, result.signal);
}

process.exit(result.status ?? 1);
