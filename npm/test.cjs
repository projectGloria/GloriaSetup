"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");
const { pathToFileURL } = require("node:url");

const gloriaSetup = require("./index.cjs");
const { packPath, stubPath, buildArgs, buildCheckArgs, check, inspect } = gloriaSetup;

test("exports absolute paths to bundled binaries", () => {
  assert.equal(packPath, path.join(__dirname, "bin", "gloria-pack.exe"));
  assert.equal(stubPath, path.join(__dirname, "bin", "gloria-stub.exe"));
});

test("ESM wrapper exposes the same API as CJS", async () => {
  const esm = await import(pathToFileURL(path.join(__dirname, "index.mjs")).href);
  assert.equal(esm.packPath, packPath);
  assert.equal(esm.stubPath, stubPath);
  assert.equal(esm.buildArgs, buildArgs);
  assert.equal(esm.pack, gloriaSetup.pack);
  assert.equal(esm.inspect, inspect);
  assert.equal(esm.check, check);
  assert.equal(esm.buildCheckArgs, buildCheckArgs);
  assert.equal(esm.default.packPath, packPath);
});

test("buildArgs maps all PackOptions fields and defaults --stub", () => {
  assert.throws(() => buildArgs(), /options object/);
  assert.throws(() => buildArgs({}), /appDir is required/);

  const minimal = buildArgs({ appDir: "app" });
  assert.deepEqual(minimal, ["--app", "app", "--stub", stubPath]);

  const full = buildArgs({
    appDir: "app",
    manifest: "gloria.toml",
    stub: "custom-stub.exe",
    version: "1.2.3",
    icon: "app.ico",
    allowDefaultIcon: true,
    outDir: "dist",
    level: 15,
    webview2Bootstrapper: "MicrosoftEdgeWebview2Setup.exe",
    signPfx: "cert.pfx",
    timestampUrl: "http://timestamp.digicert.com",
    signtool: "signtool.exe",
  });
  assert.deepEqual(full, [
    "--app", "app",
    "--manifest", "gloria.toml",
    "--stub", "custom-stub.exe",
    "--version", "1.2.3",
    "--icon", "app.ico",
    "--allow-default-icon",
    "--out", "dist",
    "--level", "15",
    "--webview2-bootstrapper", "MicrosoftEdgeWebview2Setup.exe",
    "--sign-pfx", "cert.pfx",
    "--timestamp-url", "http://timestamp.digicert.com",
    "--signtool", "signtool.exe",
  ]);
});

test("buildCheckArgs maps CheckOptions", () => {
  assert.deepEqual(buildCheckArgs(), ["--check"]);
  assert.throws(() => buildCheckArgs(null), /options object/);
  assert.deepEqual(buildCheckArgs({ manifest: "gloria.toml", version: "v1.2.3", icon: "app.ico", appDir: "app" }), [
    "--check",
    "--manifest", "gloria.toml",
    "--version", "v1.2.3",
    "--icon", "app.ico",
    "--app", "app",
  ]);
});

test("package.json bin entries are normalized and executable", () => {
  const pkg = JSON.parse(fs.readFileSync(path.join(__dirname, "package.json"), "utf8"));
  assert.ok(pkg.bin && typeof pkg.bin === "object");
  for (const [cmd, target] of Object.entries(pkg.bin)) {
    assert.equal(typeof target, "string", `bin[${cmd}] must be a string`);
    assert.ok(!target.startsWith("./"), `bin[${cmd}] must not start with './': ${target}`);
    assert.ok(!target.includes("\\"), `bin[${cmd}] must use forward slashes: ${target}`);
    const fullPath = path.join(__dirname, target);
    assert.ok(fs.existsSync(fullPath), `bin[${cmd}] target missing: ${target}`);
    const content = fs.readFileSync(fullPath, "utf8");
    assert.ok(
      content.startsWith("#!/usr/bin/env node\n"),
      `bin[${cmd}] target must start with '#!/usr/bin/env node' (LF)`
    );
  }
});

test("bundled CLI and inspect work when binaries are staged", {
  skip: process.platform !== "win32" || !fs.existsSync(packPath) || !fs.existsSync(stubPath),
}, () => {
  const help = spawnSync(process.execPath, [path.join(__dirname, "cli.cjs"), "--help"], {
    encoding: "utf8",
  });
  assert.equal(help.status, 0, help.stderr);
  assert.match(help.stdout, /gloria-pack: build a GloriaSetup installer/);

  const info = inspect(stubPath);
  assert.match(info, /bare stub/);

  const dummy = path.join(__dirname, "..", "examples", "dummy", "gloria.toml");
  if (fs.existsSync(dummy)) {
    assert.equal(check({ manifest: dummy }).status, 0);
    assert.throws(() => check({ manifest: path.join(__dirname, "no-such-gloria.toml") }), /--check exited/);
  }
});
