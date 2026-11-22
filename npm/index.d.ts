import type { StdioOptions } from "node:child_process";

/** Absolute path to the bundled `gloria-pack.exe` binary. */
export declare const packPath: string;

/** Absolute path to the bundled `gloria-stub.exe` binary. */
export declare const stubPath: string;

export interface PackOptions {
  /** Folder with the app's built files (`main_exe` from `gloria.toml` must be inside). */
  appDir: string;
  /** Path to `gloria.toml` (defaults to `<appDir>/../gloria.toml`, then `./gloria.toml`). */
  manifest?: string;
  /** Custom `gloria-stub.exe` path (defaults to the bundled `stubPath`). */
  stub?: string;
  /** Overrides `version` in `gloria.toml` (leading `v` is stripped). */
  version?: string;
  /** Overrides `icon` in `gloria.toml` (`.ico` or `.png`). */
  icon?: string;
  /** Keep the default GloriaSetup icon and mark the build `TEST BUILD`. */
  allowDefaultIcon?: boolean;
  /** Output directory for `<Name>_<version>_x64-setup.exe` (default: `dist`). */
  outDir?: string;
  /** Zstd compression level (`1`–`22`, default: `19`). */
  level?: number;
  /** Path to `MicrosoftEdgeWebview2Setup.exe` for `needs_webview2` apps. */
  webview2Bootstrapper?: string;
  /** Path to a code-signing `.pfx` certificate. */
  signPfx?: string;
  /** Password for `signPfx` (passed via `GLORIA_SIGN_PASSWORD`). */
  signPassword?: string;
  /** RFC 3161 timestamp server URL (e.g. `http://timestamp.digicert.com`). */
  timestampUrl?: string;
  /** Explicit path to `signtool.exe`. */
  signtool?: string;
  /** Working directory for `gloria-pack`. */
  cwd?: string;
  /** Child process stdio configuration (default: `"pipe"`). */
  stdio?: StdioOptions;
}

export interface PackResult {
  status: number;
  stdout: string;
  stderr: string;
  args: string[];
}

export declare function buildArgs(options: PackOptions): string[];
export declare function pack(options: PackOptions): PackResult;
export declare function inspect(exePath: string): string;

declare const gloriaSetup: {
  packPath: string;
  stubPath: string;
  buildArgs: typeof buildArgs;
  pack: typeof pack;
  inspect: typeof inspect;
};

export default gloriaSetup;
