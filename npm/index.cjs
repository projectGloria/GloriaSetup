"use strict";

const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const binDir = path.join(__dirname, "bin");
const packPath = path.join(binDir, "gloria-pack.exe");
const stubPath = path.join(binDir, "gloria-stub.exe");

function ensureBinary(filePath, name) {
  if (process.platform !== "win32") {
    throw new Error(`gloria-setup: ${name} only runs on Windows (win32 x64).`);
  }
  if (!fs.existsSync(filePath)) {
    throw new Error(
      `gloria-setup: ${name} not found at ${filePath}. Reinstall gloria-setup or stage binaries with tools/stage-npm.ps1.`
    );
  }
}

/**
 * Build CLI arguments for gloria-pack from an options object.
 * @param {import("./index.d.ts").PackOptions} options
 * @returns {string[]}
 */
function buildArgs(options) {
  if (!options || typeof options !== "object") {
    throw new TypeError("gloria-setup: pack(options) requires an options object.");
  }
  if (!options.appDir || typeof options.appDir !== "string") {
    throw new TypeError("gloria-setup: options.appDir is required.");
  }

  const args = ["--app", options.appDir];
  if (options.manifest) args.push("--manifest", options.manifest);
  args.push("--stub", options.stub || stubPath);
  if (options.version) args.push("--version", String(options.version));
  if (options.icon) args.push("--icon", options.icon);
  if (options.allowDefaultIcon) args.push("--allow-default-icon");
  if (options.outDir) args.push("--out", options.outDir);
  if (options.level !== undefined) args.push("--level", String(options.level));
  if (options.webview2Bootstrapper) {
    args.push("--webview2-bootstrapper", options.webview2Bootstrapper);
  }
  if (options.signPfx) args.push("--sign-pfx", options.signPfx);
  if (options.timestampUrl) args.push("--timestamp-url", options.timestampUrl);
  if (options.signtool) args.push("--signtool", options.signtool);
  return args;
}

/**
 * Pack a Windows per-user setup using the bundled gloria-pack.exe and gloria-stub.exe.
 * @param {import("./index.d.ts").PackOptions} options
 * @returns {import("./index.d.ts").PackResult}
 */
function pack(options) {
  ensureBinary(packPath, "gloria-pack.exe");
  const effectiveStub = (options && options.stub) || stubPath;
  if (effectiveStub === stubPath) {
    ensureBinary(stubPath, "gloria-stub.exe");
  }

  const args = buildArgs(options);
  const env = { ...process.env };
  if (options.signPassword !== undefined) {
    env.GLORIA_SIGN_PASSWORD = options.signPassword;
  }

  const stdio = options.stdio || "pipe";
  const result = spawnSync(packPath, args, {
    cwd: options.cwd,
    env,
    encoding: "utf8",
    stdio,
  });

  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    const detail = (result.stderr || result.stdout || "").trim();
    throw new Error(
      `gloria-pack exited with code ${result.status}${detail ? `: ${detail}` : ""}`
    );
  }

  return {
    status: result.status,
    stdout: result.stdout || "",
    stderr: result.stderr || "",
    args,
  };
}

/**
 * Inspect icons, version info, and payload metadata of a built setup or stub exe.
 * @param {string} exePath
 * @returns {string}
 */
function inspect(exePath) {
  if (!exePath || typeof exePath !== "string") {
    throw new TypeError("gloria-setup: inspect(exePath) requires a file path.");
  }
  ensureBinary(packPath, "gloria-pack.exe");

  const result = spawnSync(packPath, ["--inspect", exePath], {
    encoding: "utf8",
    stdio: "pipe",
  });

  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    const detail = (result.stderr || result.stdout || "").trim();
    throw new Error(
      `gloria-pack --inspect exited with code ${result.status}${detail ? `: ${detail}` : ""}`
    );
  }
  return result.stdout || "";
}

module.exports = {
  packPath,
  stubPath,
  buildArgs,
  pack,
  inspect,
};
