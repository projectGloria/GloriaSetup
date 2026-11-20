// GloriaSetup clickable mockups. Pure HTML/CSS/JS, no build step. Open index.html in a browser.
(() => {
  const USER = "C:\\Users\\you";
  const APPS = {
    omni: {
      name: "OmniConvert", version: "0.2.0", installed: "0.1.0", newer: "0.3.0",
      accent: "#f59e0b", icon: "assets/omniconvert.png", exe: "omniconvert.exe",
      desc: "Universal offline media & document converter",
      size: "48.2 MB", free: "212 GB",
      data: [["%APPDATA%\\com.omniconvert.app", "12.4 MB"], ["%LOCALAPPDATA%\\com.omniconvert.app", "186 MB"]],
      dataTotal: "198 MB",
      legacy: { type: "NSIS", scope: "all users", path: "C:\\Program Files\\OmniConvert", admin: true },
      files: ["resources\\ffmpeg.exe", "resources\\pandoc.exe", "omniconvert.exe", "resources\\pdfium.dll", "uninstall.exe"],
    },
    theme: {
      name: "GloriaTheme", version: "2.0.1", installed: "2.0.0", newer: "2.1.0",
      accent: "#dc143c", icon: "assets/gloriatheme.jpg", exe: "GloriaTheme.exe",
      desc: "Cyberpunk neon bloom & VS Code control panel",
      size: "6.8 MB", free: "212 GB",
      data: [["%APPDATA%\\GloriaTheme", "1.1 MB"]],
      dataTotal: "1.1 MB",
      legacy: { type: "Inno Setup", scope: "this account", path: USER + "\\AppData\\Local\\Programs\\GloriaTheme", admin: false },
      files: ["themes\\gloria-neon.json", "GloriaTheme.exe", "assets\\gloria-icons.woff", "uninstall.exe"],
    },
    sample: {
      name: "Sample App", version: "1.0.0", installed: "0.9.0", newer: "1.1.0",
      accent: "#4caf50", icon: "../../assets/brand/GloriaSetup.png", exe: "SampleApp.exe", test: true,
      desc: "Bare preview without a manifest",
      size: "1.2 MB", free: "212 GB",
      data: [["%APPDATA%\\SampleApp", "24 KB"]],
      dataTotal: "24 KB",
      legacy: { type: "MSI", scope: "all users", path: "C:\\Program Files (x86)\\Sample App", admin: true },
      files: ["SampleApp.exe", "uninstall.exe"],
    },
  };

  const SCREENS = [
    ["welcome", "Welcome", "fresh"],
    ["options", "Options", "fresh"],
    ["options-error", "Options · bad folder", "fresh"],
    ["progress", "Progress", "fresh"],
    ["done", "Done", "fresh"],
    ["update", "Update", "existing"],
    ["update-running", "Update · app running", "existing"],
    ["reinstall", "Reinstall", "existing"],
    ["downgrade", "Downgrade warning", "existing"],
    ["migrate", "Upgrade from old installer", "existing"],
    ["uninstall", "Uninstall", "uninstall"],
    ["uninstall-progress", "Uninstall · progress", "uninstall"],
    ["uninstall-done", "Uninstall · done", "uninstall"],
    ["error", "Error", "other"],
  ];

  const S = {
    app: "omni", lang: "en", screen: "welcome", scale: 1, corners: "square", primary: "cream", credit: true, view: "single",
    folder: null, desktop: false, launch: true, purge: false, choice: "repair", confirmDown: false,
    modal: null, pct: 0, flow: "install",
  };

  // ───────── helpers ─────────
  const app = () => APPS[S.app];
  const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
  const t = (key, vars = {}) => {
    const dict = I18N[S.lang] || I18N.en;
    let s = dict[key] ?? I18N.en[key] ?? key;
    const all = { name: app().name, version: app().version, exe: app().exe, ...vars };
    return s.replace(/\{(\w+)\}/g, (_, k) => all[k] ?? "");
  };
  const T = (key, vars) => esc(t(key, vars));
  const defaultFolder = () => `${USER}\\AppData\\Local\\Programs\\${app().name.replace(/\s/g, "")}`;
  const hexRgb = (h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));
  const onAccent = (h) => {
    const [r, g, b] = hexRgb(h).map((v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; });
    const L = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    return (1.05) / (L + 0.05) > (L + 0.05) / 0.05 ? "#ffffff" : "#0b0b0c";
  };

  const ICONS = {
    check: '<path d="M20 6 9 17l-5-5"/>',
    x: '<path d="M18 6 6 18M6 6l12 12"/>',
    minus: '<path d="M5 12h14"/>',
    sliders: '<path d="M4 21v-7M4 10V3M12 21v-9M12 8V3M20 21v-5M20 12V3M1 14h6M9 8h6M17 16h6"/>',
    arrow: '<path d="M5 12h14M13 6l6 6-6 6"/>',
    folder: '<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>',
    shield: '<path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/><path d="m9 12 2 2 4-4"/>',
    alert: '<path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3"/><path d="M12 9v4M12 17h.01"/>',
    info: '<circle cx="12" cy="12" r="10"/><path d="M12 16v-4M12 8h.01"/>',
    play: '<path d="m6 3 14 9-14 9V3z"/>',
    trash: '<path d="M3 6h18M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/>',
    refresh: '<path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/><path d="M21 3v5h-5M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/><path d="M8 16H3v5"/>',
    lock: '<rect width="18" height="11" x="3" y="11" rx="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/>',
    file: '<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/>',
    upshield: '<path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/>',
  };
  const ic = (n, cls = "") => `<svg class="i ${cls}" viewBox="0 0 24 24">${ICONS[n]}</svg>`;

  // ───────── components ─────────
  const appIcon = (size, extra = "") => {
    const a = app();
    let badge = "";
    if (extra.includes("ok")) badge = `<span class="badge-ok">${ic("check")}</span>`;
    if (extra.includes("err")) badge = `<span class="badge-x">${ic("x")}</span>`;
    return `<div class="appicon ${size < 80 ? "sm" : ""} ${extra.includes("gray") ? "gray" : ""}">
      <img src="${a.icon}" width="${size}" height="${size}" alt="">${badge}</div>`;
  };
  const cb = (act, on, label, { cls = "", sub = "", tag = "" } = {}) =>
    `<div class="cb ${on ? "on" : ""} ${cls}" data-act="${act}" role="checkbox" aria-checked="${on}" tabindex="0">
      <span class="box">${ic("check")}</span>
      <span>${label}${tag ? `<span class="tag">${tag}</span>` : ""}${sub ? `<small>${sub}</small>` : ""}</span></div>`;
  const dataBlock = (labelKey, on) => {
    const a = app();
    return `<div class="data ${on ? "on" : ""}">
      ${cb("purge", on, T(labelKey), { cls: "danger" })}
      <ul>${a.data.map(([p, s]) => `<li><span>${esc(p)}</span><b>${s}</b></li>`).join("")}</ul>
      <div class="total">${T(on ? "data.total" : "data.totalKeep", { size: a.dataTotal })}</div></div>`;
  };
  const btn = (act, label, cls = "", icon = "", disabled = false) =>
    `<button class="btn ${cls}" data-act="${act}" ${disabled ? "disabled" : ""}>${icon ? ic(icon) : ""}${label}</button>`;
  const credit = () => (S.credit ? `<div class="credit"><img src="../../assets/brand/GloriaSetup.png" alt="">${T("credit")}</div>` : "");
  const hero = (iconSize, iconExtra, inner, cls = "") =>
    `<div class="hero ${cls}">${appIcon(iconSize, iconExtra)}<div style="flex:1;min-width:0">${inner}</div></div>`;

  const validate = (path) => {
    const p = path.trim();
    if (/^[a-z]:\\?$/i.test(p)) return { err: "options.errRoot" };
    if (/^[a-z]:\\(program files|program files \(x86\)|windows|programdata)(\\|$)/i.test(p)) return { err: "options.errProtected" };
    if (/\\(documents|desktop|downloads)$/i.test(p)) return { final: `${p}\\${app().name.replace(/\s/g, "")}`, appended: true };
    return { final: p };
  };

  const STEPS_INSTALL = ["step.prepare", "step.copy", "step.shortcuts", "step.register"];
  const STEPS_UNINSTALL = ["step.close", "step.files", "step.data", "step.entry"];
  const stepsHtml = (keys, pct, skip = []) => {
    const idx = pct >= 100 ? keys.length : pct < 8 ? 0 : pct < 78 ? 1 : pct < 90 ? 2 : 3;
    return `<div class="steps">${keys.map((k, i) => {
      const st = skip.includes(k) ? "" : i < idx ? "done" : i === idx ? "act" : "";
      const style = skip.includes(k) ? ' style="text-decoration:line-through;opacity:.5"' : "";
      return `<div class="step ${st}"${style}><span class="dot">${st === "done" ? ic("check") : ""}</span>${T(k)}</div>`;
    }).join("")}</div>`;
  };

  // ───────── screens ─────────
  function screen(id, st) {
    const a = app();
    const folder = st.folder ?? defaultFolder();
    switch (id) {
      case "welcome":
        return {
          body: hero(96, "", `
            <div class="kicker">${T("welcome.kicker")}</div>
            <div class="h1">${esc(a.name)}</div>
            <div class="chips"><span class="chip a">V${a.version}</span><span class="chip badge">BY GLORIA</span></div>
            <p class="p">${esc(a.desc)}</p>
            <div class="facts">
              <div class="lbl">${T("location")}</div><div class="v">${esc(folder)}</div>
              <div class="lbl">${T("size")}</div><div class="v">${a.size}</div>
            </div>`),
          foot: `${btn("go:options", T("welcome.options"), "ghost", "sliders")}<div class="sp"></div>${btn("install", T("welcome.install"), "primary")}`,
        };

      case "options":
      case "options-error": {
        const path = id === "options-error" && st.folder == null ? "C:\\Program Files\\" + a.name : folder;
        const v = validate(path);
        const hint = v.err
          ? `<div class="hint err" id="hint">${ic("alert")}<span>${T(v.err)}</span></div>`
          : v.appended
          ? `<div class="hint info" id="hint">${ic("info")}<span>${T("options.appended")}<br>${T("options.final")} <span class="mono">${esc(v.final)}</span></span></div>`
          : `<div class="hint" id="hint">${ic("folder")}<span>${T("options.space", { req: a.size, free: a.free, drive: path.slice(0, 2).toUpperCase() || "C:" })}</span></div>`;
        return {
          body: hero(64, "", `
            <div class="kicker">${T("options.kicker")}</div>
            <div class="h2">${T("options.title")}</div>
            <div style="margin-top:18px" class="lbl">${T("options.location")}</div>
            <div class="field ${v.err ? "err" : ""}" id="field"><input id="folder" value="${esc(path)}" spellcheck="false">${btn("browse", T("browse"), "", "folder")}</div>
            ${hint}
            <div class="try"><span class="lbl" style="align-self:center">${T("try")}</span>
              <button data-act="try:default">Default</button><button data-act="try:pf">Program Files</button><button data-act="try:docs">Documents</button><button data-act="try:root">D:\\</button></div>
            <div class="opt-grid">
              <div><div class="lbl">${T("options.shortcuts")}</div>
                ${cb("noop", true, T("options.startMenu"), { cls: "locked", tag: T("options.always").toUpperCase() })}
                ${cb("desktop", st.desktop, T("options.desktop"))}</div>
              <div><div class="lbl">${T("options.after")}</div>
                ${cb("launch", st.launch, T("options.launch"))}</div>
            </div>`),
          foot: `${btn("go:welcome", T("back"))}<div class="sp"></div>${btn("install", T("welcome.install"), "primary", "", !!v.err)}`,
          err: !!v.err,
        };
      }

      case "progress": {
        const pct = st.pct;
        const titleKey = st.flow === "update" ? "update.title" : "progress.title";
        const file = a.files[Math.min(a.files.length - 1, Math.floor((pct / 100) * a.files.length))];
        return {
          body: hero(64, "", `
            <div class="kicker">${T(st.flow === "update" ? "update.kicker" : "progress.kicker")}</div>
            <div class="h2">${T(titleKey)}</div>
            <div style="margin-top:26px" class="pct" id="pct">${Math.floor(pct)}<small>%</small></div>
            <div class="bar"><i id="bar" style="width:${pct}%"></i></div>
            <div class="file" id="file">${esc(folder)}\\${esc(file)}</div>
            <div id="steps">${stepsHtml(STEPS_INSTALL, pct)}</div>`),
          foot: `<div class="sp"></div>${btn("go:welcome", T("cancel"))}`,
        };
      }

      case "done": {
        const body = st.flow === "update" ? t("done.updated") : t("done.body");
        return {
          body: hero(96, "ok", `
            <div class="kicker">${T("done.kicker")}</div>
            <div class="h1">${T("done.title")}</div>
            <p class="p">${esc(body)}</p>
            <div class="facts">
              <div class="lbl">${T("version")}</div><div class="v">${a.version}</div>
              <div class="lbl">${T("location")}</div><div class="v">${esc(folder)}</div>
            </div>`),
          foot: `${credit()}<div class="sp"></div>${btn("go:welcome", T("close"))}${btn("go:welcome", T("done.launch"), "primary", "play")}`,
        };
      }

      case "update":
      case "update-running":
        return {
          body: hero(64, "", `
            <div class="kicker">${T("update.kicker")}</div>
            <div class="h2">${T("update.title")}</div>
            <div class="vers">
              <div><span class="lbl">${T("update.installed")}</span><span class="n">${a.installed}</span></div>
              <div class="arrow">${ic("arrow")}</div>
              <div><span class="lbl">${T("update.new")}</span><span class="n a">${a.version}</span></div>
            </div>
            <div class="facts" style="margin-top:20px">
              <div class="lbl">${T("data")}</div><div class="v ok">${ic("shield")}${T("update.kept")}</div>
              <div class="lbl">${T("location")}</div><div class="v">${esc(folder)}</div>
            </div>
            ${id === "update-running" ? `<div class="note warn">${ic("alert")}<span>${T("update.running")}</span></div>` : ""}`),
          foot: `<div class="sp"></div>${btn("go:welcome", T("cancel"))}${btn(id === "update-running" ? "modal:ask" : "update", T("update.btn"), "primary", "refresh")}`,
          modal: id === "update-running" ? st.modal : null,
        };

      case "reinstall":
        return {
          body: hero(64, "", `
            <div class="kicker">${T("reinstall.kicker")}</div>
            <div class="h2">${T("reinstall.title")}</div>
            <div class="cards">
              <div class="card ${st.choice === "repair" ? "on" : ""}" data-act="choice:repair"><span class="rd"></span><div><b>${T("reinstall.repair")}</b><span>${T("reinstall.repairDesc")}</span></div></div>
              <div class="card ${st.choice === "remove" ? "on" : ""}" data-act="choice:remove"><span class="rd"></span><div><b>${T("reinstall.remove")}</b><span>${T("reinstall.removeDesc")}</span></div></div>
            </div>
            ${st.choice === "repair" ? dataBlock("reinstall.clean", st.purge) : ""}`),
          foot: `<div class="sp"></div>${btn("go:welcome", T("cancel"))}${btn(st.choice === "repair" ? "reinstall" : "go:uninstall", T("continue"), "primary")}`,
        };

      case "downgrade":
        return {
          body: hero(64, "", `
            <div class="kicker warn">${T("down.kicker")}</div>
            <div class="h2">${T("down.title")}</div>
            <div class="vers" style="margin-top:16px">
              <div><span class="lbl">${T("update.installed")}</span><span class="n">${a.newer}</span></div>
              <div class="arrow">${ic("arrow")}</div>
              <div><span class="lbl">${T("version")}</span><span class="n w">${a.version}</span></div>
            </div>
            <p class="p" style="font-size:12px;margin-top:10px">${T("down.body")}</p>
            ${dataBlock("data.remove", st.purge)}
            <div style="margin-top:10px">${cb("confirmDown", st.confirmDown, T("down.confirm"))}</div>`, "tight"),
          foot: `<div class="sp"></div>${btn("go:welcome", T("cancel"))}${btn("downgrade", T("down.btn"), "primary", "", !st.confirmDown)}`,
        };

      case "migrate": {
        const L = a.legacy;
        return {
          body: hero(64, "", `
            <div class="kicker">${T("mig.kicker")}</div>
            <div class="h2">${T("mig.title")}</div>
            <div class="mig">
              <div><span class="num">1</span><div class="sp">${T("mig.step1")}<small>${a.installed} · ${L.type} · ${esc(L.scope)}</small></div>
                ${L.admin ? `<span class="chip" style="color:var(--warn);border-color:var(--warn-line);background:var(--warn-bg);display:flex;gap:5px;align-items:center">${ic("upshield")}${T("mig.admin").toUpperCase()}</span>` : ""}</div>
              <div><span class="num">2</span><div class="sp">${T("mig.step2")}<small>${esc(folder)}</small></div></div>
            </div>
            ${L.admin ? `<div class="hint">${ic("info")}<span>${T("mig.adminNote")}</span></div>` : ""}
            ${dataBlock("data.remove", st.purge)}`, "tight"),
          foot: `<div class="sp"></div>${btn("go:welcome", T("cancel"))}${btn("migrate", T("mig.btn"), "primary", L.admin ? "upshield" : "")}`,
        };
      }

      case "uninstall":
        return {
          danger: true, uninstaller: true,
          body: hero(64, "", `
            <div class="kicker danger">${T("un.kicker")}</div>
            <div class="h2">${T("un.title")}</div>
            <p class="p">${T("un.body")}</p>
            <div class="facts" style="margin-top:16px">
              <div class="lbl">${T("location")}</div><div class="v">${esc(folder)}</div>
            </div>
            ${dataBlock("data.remove", st.purge)}`),
          foot: `<div class="sp"></div>${btn("go:welcome", T("cancel"))}${btn("uninstall", T("un.btn"), "danger", "trash")}`,
        };

      case "uninstall-progress": {
        const pct = st.pct;
        return {
          danger: true, uninstaller: true,
          body: hero(64, "", `
            <div class="kicker danger">${T("unp.kicker")}</div>
            <div class="h2">${T("unp.title")}</div>
            <div style="margin-top:26px" class="pct" id="pct">${Math.floor(pct)}<small>%</small></div>
            <div class="bar"><i id="bar" style="width:${pct}%"></i></div>
            <div class="file" id="file">${esc(folder)}\\${esc(a.files[Math.min(a.files.length - 1, Math.floor((pct / 100) * a.files.length))])}</div>
            <div id="steps">${stepsHtml(STEPS_UNINSTALL, pct, st.purge ? [] : ["step.data"])}</div>`),
          foot: `<div class="sp"></div>`,
        };
      }

      case "uninstall-done":
        return {
          uninstaller: true,
          body: hero(96, "gray", `
            <div class="kicker">${T("und.kicker")}</div>
            <div class="h1" style="font-size:28px">${T("und.title")}</div>
            <p class="p">${T(st.purge ? "und.purged" : "und.kept")}</p>
            ${st.purge ? "" : `<div class="facts"><div class="lbl">${T("data")}</div><div class="v ok">${ic("shield")}${esc(a.data[0][0])}${a.data.length > 1 ? " +" + (a.data.length - 1) : ""}</div></div>`}`),
          foot: `${credit()}<div class="sp"></div>${btn("go:welcome", T("close"), "primary")}`,
        };

      case "error":
        return {
          danger: true,
          body: hero(64, "err", `
            <div class="kicker danger">${T("err.kicker")}</div>
            <div class="h2">${T("err.title")}</div>
            <p class="p">${T("err.body")}</p>
            <div class="code">
              <div><span class="k">${T("err.code").toUpperCase()}</span><span class="v">3</span></div>
              <div><span class="k">${T("err.reason").toUpperCase()}</span><span class="v">${T("err.reasonV")}</span></div>
              <div><span class="k">${T("err.log").toUpperCase()}</span><span class="v">%TEMP%\\GloriaSetup-${esc(a.name.replace(/\s/g, ""))}-20261120-2207.log · <a data-act="noop">${T("err.open")}</a></span></div>
            </div>`),
          foot: `${btn("noop", T("err.log"), "ghost", "file")}<div class="sp"></div>${btn("go:welcome", T("close"))}${btn("retry", T("err.retry"), "primary", "refresh")}`,
        };
    }
  }

  function modalHtml(kind) {
    if (!kind) return "";
    const a = app();
    const proc = `${defaultFolder()}\\${a.exe}`;
    const body =
      kind === "ask" ? `<p class="p" style="margin-top:8px">${T("running.body")}</p><div class="proc"><i></i>${esc(proc)} · PID 14212</div>`
      : kind === "waiting" ? `<p class="p" style="margin-top:8px">${T("running.waiting")}</p><div class="proc closing"><i></i>${esc(proc)} · PID 14212</div>`
      : `<p class="p" style="margin-top:8px">${T("running.stuck")}</p><div class="proc closing"><i></i>${esc(proc)} · PID 14212</div>`;
    const foot =
      kind === "ask" ? `${btn("modal:", T("cancel"))}${btn("modal:waiting", T("running.closeBtn"), "primary")}`
      : kind === "waiting" ? `${btn("modal:", T("cancel"))}<button class="btn primary" disabled><span class="spin"></span>${T("running.closeBtn")}</button>`
      : `${btn("modal:", T("cancel"))}${btn("force", T("running.force"), "danger")}`;
    return `<div class="scrim"><div class="dlg"><div class="dlg-b"><div class="kicker warn">${esc(a.name)}</div><div class="h2" style="font-size:19px">${T("running.title")}</div>${body}</div><div class="dlg-f">${foot}</div></div></div>`;
  }

  function windowHtml(id, st, { animate = true } = {}) {
    const a = app();
    const sc = screen(id, st);
    const title = sc.uninstaller ? t("uninstaller") : t("setup");
    const rgb = hexRgb(a.accent).join(", ");
    const style = `--a:${a.accent};--a-rgb:${rgb};--on-a:${onAccent(a.accent)}`;
    return `<div class="win ${st.corners === "rounded" ? "rounded" : ""} ${sc.danger ? "danger" : ""} ${st.primary === "accent" ? "accent-primary" : ""}" style="${style}" lang="${st.lang}">
      <div class="glow-bg"></div>
      <div class="tb">
        <div class="tb-title"><img src="${a.icon}" alt=""><span>${esc(title)}</span>${a.test ? '<span class="test">TEST BUILD</span>' : ""}</div>
        <button title="${T("minimize")}">${ic("minus")}</button><button class="x" title="${T("close")}" data-act="go:welcome">${ic("x")}</button>
      </div>
      <div class="body"><div class="page" ${animate ? "" : 'style="animation:none"'}>
        <div class="content">${sc.body}</div>
        <div class="foot">${sc.foot}</div>
      </div>${modalHtml(sc.modal)}</div>
    </div>`;
  }

  // ───────── control panel ─────────
  const seg = (key, opts) => `<div class="seg">${opts.map(([v, l]) => `<button data-set="${key}:${v}" class="${String(S[key]) === String(v) ? "on" : ""}">${l}</button>`).join("")}</div>`;
  function renderCtl() {
    const groups = { fresh: "Fresh install", existing: "Existing install", uninstall: "Uninstall", other: "Other" };
    let list = "";
    for (const g of Object.keys(groups)) {
      list += `<h4 style="margin-top:12px">${groups[g]}</h4><div class="screens">${SCREENS.filter((s) => s[2] === g)
        .map(([id, label]) => `<button data-screen="${id}" class="${S.screen === id && S.view === "single" ? "on" : ""}">${label}<span>${id}</span></button>`).join("")}</div>`;
    }
    document.getElementById("ctl").innerHTML = `
      <div class="ctl-brand"><img src="../../assets/brand/GloriaSetup.png" alt=""><div><b>GLORIASETUP</b><small>Mockups · v0</small></div></div>
      <div><h4>View</h4>${seg("view", [["single", "Clickable"], ["grid", "All screens"]])}</div>
      <div><h4>App theme</h4>${seg("app", [["omni", "OmniConvert"], ["theme", "GloriaTheme"], ["sample", "No manifest"]])}</div>
      <div><h4>Language</h4>${seg("lang", [["en", "EN"], ["fr", "FR"], ["es", "ES"], ["tr", "TR"]])}</div>
      <div><h4>Display scale</h4>${seg("scale", [["1", "100%"], ["1.25", "125%"], ["1.5", "150%"], ["2", "200%"]])}</div>
      <div style="border-top:1px solid var(--line);padding-top:14px"><p class="ctl-note" style="margin-bottom:10px"><b style="color:var(--text)">Your call:</b> these three are open design choices.</p>
        <h4>Window corners</h4>${seg("corners", [["square", "Square (launcher)"], ["rounded", "Win11 rounded"]])}
        <h4 style="margin-top:10px">Primary button</h4>${seg("primary", [["cream", "Cream (launcher)"], ["accent", "App accent"]])}
        <h4 style="margin-top:10px">GloriaSetup credit on done</h4>${seg("credit", [["true", "Show"], ["false", "Hide"]])}</div>
      <div>${list}</div>
      <p class="ctl-note">Buttons inside the window navigate like the real installer. Progress is simulated. The real UI is Direct2D, 640×440 DIP; see <code>docs/design.md</code> §3 for how each effect maps.</p>`;
  }

  // ───────── stage ─────────
  let timer = null;
  function stopTimer() { if (timer) { clearInterval(timer); timer = null; } }

  function renderStage() {
    const stage = document.getElementById("stage");
    stage.classList.toggle("grid", S.view === "grid");
    stopTimer();
    if (S.view === "grid") {
      const st = { ...S, pct: 62, modal: null };
      stage.innerHTML = `<div class="overview">${SCREENS.map(([id, label]) => {
        const s2 = { ...st, modal: id === "update-running" ? "ask" : null, purge: id === "uninstall-done" ? false : st.purge };
        return `<figure><figcaption>${label}</figcaption><div class="thumb" data-screen="${id}">${windowHtml(id, s2, { animate: false })}</div></figure>`;
      }).join("")}</div>`;
      return;
    }
    stage.innerHTML = `<div class="stage-inner" style="zoom:${S.scale}">${windowHtml(S.screen, S)}</div>`;
    if (S.screen === "progress" || S.screen === "uninstall-progress") runProgress();
  }

  function runProgress() {
    const done = S.screen === "progress" ? "done" : "uninstall-done";
    timer = setInterval(() => {
      S.pct = Math.min(100, S.pct + 1.6 + Math.random() * 2.2);
      const a = app();
      const folder = S.folder ?? defaultFolder();
      const el = (id) => document.getElementById(id);
      if (!el("pct")) return stopTimer();
      el("pct").innerHTML = `${Math.floor(S.pct)}<small>%</small>`;
      el("bar").style.width = S.pct + "%";
      el("file").textContent = `${folder}\\${a.files[Math.min(a.files.length - 1, Math.floor((S.pct / 100) * a.files.length))]}`;
      el("steps").innerHTML = stepsHtml(S.screen === "progress" ? STEPS_INSTALL : STEPS_UNINSTALL, S.pct, S.screen === "uninstall-progress" && !S.purge ? ["step.data"] : []);
      if (S.pct >= 100) { stopTimer(); setTimeout(() => go(done), 450); }
    }, 70);
  }

  function go(screen) {
    S.screen = screen;
    if (screen === "progress" || screen === "uninstall-progress") S.pct = 0;
    if (screen === "welcome") { S.flow = "install"; S.modal = null; S.confirmDown = false; }
    if (screen === "update-running") S.modal = null;
    syncHash();
    render();
  }

  function render() { renderCtl(); renderStage(); }
  function syncHash() { history.replaceState(null, "", `#${S.screen}/${S.app}/${S.lang}`); }

  // ───────── events ─────────
  document.addEventListener("click", (e) => {
    const setEl = e.target.closest("[data-set]");
    if (setEl) {
      const [k, v] = setEl.dataset.set.split(":");
      S[k] = k === "scale" ? Number(v) : k === "credit" ? v === "true" : v;
      if (k === "app") S.folder = null;
      syncHash();
      return render();
    }
    const scr = e.target.closest("[data-screen]");
    if (scr) {
      S.view = "single"; S.purge = false; S.folder = null; S.modal = null;
      S.flow = scr.dataset.screen.startsWith("update") ? "update" : "install";
      return go(scr.dataset.screen);
    }
    const actEl = e.target.closest("[data-act]");
    if (!actEl || actEl.hasAttribute("disabled")) return;
    const act = actEl.dataset.act;
    const [verb, arg] = act.split(":");
    switch (verb) {
      case "go": return go(arg);
      case "install": S.flow = "install"; return go("progress");
      case "update": S.flow = "update"; return go("progress");
      case "reinstall": case "downgrade": case "migrate": case "retry": S.flow = "install"; return go("progress");
      case "force": S.flow = "update"; S.modal = null; return go("progress");
      case "uninstall": return go("uninstall-progress");
      case "choice": S.choice = arg; return renderStage();
      case "desktop": case "launch": case "purge": case "confirmDown": S[verb] = !S[verb]; return renderStage();
      case "modal":
        S.modal = arg || null;
        renderStage();
        if (arg === "waiting") setTimeout(() => { if (S.modal === "waiting") { S.modal = "stuck"; renderStage(); } }, 2200);
        return;
      case "try": {
        const a = app();
        S.folder = { default: null, pf: "C:\\Program Files\\" + a.name, docs: USER + "\\Documents", root: "D:\\" }[arg];
        if (S.screen === "options-error") S.screen = "options";
        return renderStage();
      }
    }
  });
  document.addEventListener("keydown", (e) => {
    if ((e.key === " " || e.key === "Enter") && e.target.matches("[role=checkbox]")) { e.preventDefault(); e.target.click(); }
  });
  document.addEventListener("input", (e) => {
    if (e.target.id !== "folder") return;
    S.folder = e.target.value;
    if (S.screen === "options-error") S.screen = "options";
    // Re-render everything except the input, so the caret stays put.
    const tmp = document.createElement("div");
    tmp.innerHTML = windowHtml("options", S, { animate: false });
    for (const id of ["hint"]) document.getElementById(id).replaceWith(tmp.querySelector("#" + id));
    document.getElementById("field").className = tmp.querySelector("#field").className;
    const nb = tmp.querySelector('[data-act="install"]'), ob = document.querySelector('.stage [data-act="install"]');
    ob.disabled = nb.disabled;
  });

  // ───────── boot ─────────
  const [h1, h2, h3] = location.hash.slice(1).split("/");
  if (SCREENS.some((s) => s[0] === h1)) S.screen = h1;
  if (APPS[h2]) S.app = h2;
  if (I18N[h3]) S.lang = h3;
  if (S.screen.startsWith("update")) S.flow = "update";
  render();
})();
