# gloria-setup

Per-user (no admin) Windows x64 installer builder for projectGloria apps. Bundles `gloria-pack.exe` and `gloria-stub.exe` directly — no postinstall downloads required.

## Install

```bash
npm install --save-dev gloria-setup
```

Requires Windows x64 (`win32` / `x64`) and Node.js 18+.

## CLI

`gloria-pack` (aliased as `gloria-setup`) automatically uses the bundled `gloria-stub.exe` when `--stub` is omitted:

```powershell
npx gloria-pack --app path\to\built\app --manifest gloria.toml --version 1.2.0 --out dist
npx gloria-pack --inspect dist\MyApp_1.2.0_x64-setup.exe
```

For Authenticode signing, pass the PFX password via `GLORIA_SIGN_PASSWORD`:

```powershell
$env:GLORIA_SIGN_PASSWORD = '<pfx password>'
npx gloria-pack --app path\to\built\app --manifest gloria.toml --sign-pfx C:\Users\you\cert.pfx --timestamp-url http://timestamp.digicert.com
```

## JavaScript / TypeScript API

Both ESM and CommonJS are supported:

```js
import { pack, inspect, packPath, stubPath } from "gloria-setup";

const res = pack({
  appDir: "dist/app",
  manifest: "gloria.toml",
  version: "1.2.0",
  outDir: "dist",
});

console.log(res.stdout);
console.log(inspect(stubPath));
```

## Documentation

Full manifest schema (`gloria.toml`), silent installer flags (`/S`, `/D=...`, `/PURGE`), and GitHub Action docs live in the main repository: https://github.com/projectGloria/GloriaSetup
