window.__ModuleLoader__.load({ id: "dsh-agw-oauth", factory: (require) => {
var module = { exports: {} }; var exports = module.exports;
"use strict";
var __defProp = Object.defineProperty;
var __getOwnPropDesc = Object.getOwnPropertyDescriptor;
var __getOwnPropNames = Object.getOwnPropertyNames;
var __hasOwnProp = Object.prototype.hasOwnProperty;
var __export = (target, all) => {
  for (var name in all)
    __defProp(target, name, { get: all[name], enumerable: true });
};
var __copyProps = (to, from, except, desc) => {
  if (from && typeof from === "object" || typeof from === "function") {
    for (let key of __getOwnPropNames(from))
      if (!__hasOwnProp.call(to, key) && key !== except)
        __defProp(to, key, { get: () => from[key], enumerable: !(desc = __getOwnPropDesc(from, key)) || desc.enumerable });
  }
  return to;
};
var __toCommonJS = (mod) => __copyProps(__defProp({}, "__esModule", { value: true }), mod);

// src/client/index.ts
var index_exports = {};
__export(index_exports, {
  apply: () => apply,
  inject: () => inject
});
module.exports = __toCommonJS(index_exports);

// src/client/AgwSection.tsx
var import_react = require("react");
var import_jsx_runtime = require("react/jsx-runtime");
async function api(path, init) {
  const response = await fetch(path, {
    credentials: "same-origin",
    ...init,
    headers: {
      accept: "application/json",
      ...init?.body === void 0 ? {} : { "content-type": "application/json" },
      ...init?.headers
    }
  });
  let json = void 0;
  try {
    json = await response.json();
  } catch {
    json = void 0;
  }
  if (!response.ok) {
    const message = json !== null && typeof json === "object" && "error" in json && typeof json.error === "string" ? json.error : `HTTP ${response.status}`;
    throw new Error(message);
  }
  return json;
}
var page = {
  display: "flex",
  flexDirection: "column",
  gap: 16,
  maxWidth: 560,
  padding: "8px 0"
};
var titleStyle = {
  margin: 0,
  fontSize: 20,
  fontWeight: 600
};
var descStyle = {
  margin: 0,
  opacity: 0.75,
  lineHeight: 1.5
};
var fieldStyle = {
  display: "flex",
  flexDirection: "column",
  gap: 6
};
var inputStyle = {
  padding: "8px 10px",
  borderRadius: 8,
  border: "1px solid rgba(127,127,127,0.35)",
  background: "transparent",
  color: "inherit",
  fontSize: 14
};
var rowStyle = {
  display: "flex",
  alignItems: "center",
  gap: 10
};
var btnStyle = {
  padding: "8px 14px",
  borderRadius: 8,
  border: "1px solid rgba(127,127,127,0.35)",
  background: "transparent",
  color: "inherit",
  cursor: "pointer",
  fontSize: 14
};
var codeStyle = {
  fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
  fontSize: 16,
  letterSpacing: 1,
  padding: "6px 10px",
  borderRadius: 6,
  background: "rgba(127,127,127,0.12)"
};
function AgwSection(props) {
  const t = (key, fallback) => props.t?.(key) ?? fallback;
  const [origin, setOrigin] = (0, import_react.useState)("");
  const [loggedIn, setLoggedIn] = (0, import_react.useState)(false);
  const [watch, setWatch] = (0, import_react.useState)();
  const [busy, setBusy] = (0, import_react.useState)(false);
  const [error, setError] = (0, import_react.useState)();
  const [start, setStart] = (0, import_react.useState)();
  const [importText, setImportText] = (0, import_react.useState)();
  const applyStatus = (payload) => {
    setLoggedIn(payload.loggedIn === true);
    if (typeof payload.origin === "string") setOrigin(payload.origin);
    setWatch(payload.watch);
  };
  const refresh = (0, import_react.useCallback)(async () => {
    const payload = await api("/agw-oauth/status");
    applyStatus(payload);
    return payload;
  }, []);
  (0, import_react.useEffect)(() => {
    void refresh().catch((err) => {
      setError(err instanceof Error ? err.message : String(err));
    });
  }, [refresh]);
  const waiting = watch?.status === "waiting" || start !== void 0 && !loggedIn && watch?.status !== "error";
  (0, import_react.useEffect)(() => {
    if (!waiting) return;
    const timer = setInterval(() => {
      void refresh().then((payload) => {
        if (payload.loggedIn === true || payload.watch?.status === "ok" || payload.watch?.status === "error") {
          setStart(void 0);
        }
      }).catch((err) => {
        setError(err instanceof Error ? err.message : String(err));
      });
    }, 2e3);
    return () => clearInterval(timer);
  }, [waiting, refresh]);
  const persistOrigin = async () => {
    const trimmed = origin.trim();
    if (trimmed.length === 0) return;
    await api("/agw-oauth/origin", { method: "POST", body: JSON.stringify({ origin: trimmed }) });
  };
  const onLogin = async () => {
    setBusy(true);
    setError(void 0);
    try {
      await persistOrigin();
      const result = await api("/agw-oauth/login/start", { method: "POST" });
      if (result.kind === "error") {
        setError(result.text ?? result.error ?? t("error", "Something went wrong"));
        return;
      }
      setStart(result);
      await refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };
  const onImport = async () => {
    setBusy(true);
    setError(void 0);
    try {
      const report = await api("/agw-oauth/import-local", { method: "POST" });
      const lines = (report.found ?? []).map((row) => `${row.provider ?? "?"} \xB7 ${row.source ?? ""}`);
      if (report.uploaded?.status !== void 0) lines.push(`Upload HTTP ${report.uploaded.status}`);
      if (report.error) lines.push(report.error);
      setImportText(lines.join("\n") || t("importHint", "No local CLI files found."));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };
  const onLogout = async () => {
    setBusy(true);
    setError(void 0);
    try {
      await api("/agw-oauth/logout", { method: "POST" });
      setStart(void 0);
      setWatch(void 0);
      setLoggedIn(false);
      await refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };
  const openUrl = start?.openUrl ?? watch?.openUrl;
  const userCode = start?.userCode ?? watch?.userCode;
  const watchError = watch?.status === "error" ? watch.detail : void 0;
  return /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("div", { style: page, children: [
    /* @__PURE__ */ (0, import_jsx_runtime.jsx)("h1", { style: titleStyle, children: t("title", "AGW Oauth") }),
    /* @__PURE__ */ (0, import_jsx_runtime.jsx)("p", { style: descStyle, children: t("description", "\u8FDE\u63A5 AI-GateWay \u7F51\u5173\u5E76\u901A\u8FC7\u6D4F\u89C8\u5668\u5B89\u5168\u767B\u5F55\u3002") }),
    /* @__PURE__ */ (0, import_jsx_runtime.jsx)("p", { style: descStyle, children: t("importHint", "\u5DF2\u6709 ~/.codex/auth.json \u6216 ~/.claude/.credentials.json \u65F6\uFF0C\u4F18\u5148\u5BFC\u5165\uFF0C\u4E0D\u5FC5\u518D\u767B\u5F55\u3002") }),
    /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("label", { style: fieldStyle, children: [
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)("span", { children: t("originLabel", "\u7F51\u5173\u5730\u5740") }),
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)(
        "input",
        {
          style: inputStyle,
          value: origin,
          placeholder: t("originPlaceholder", "https://gw.example.com"),
          autoComplete: "off",
          spellCheck: false,
          onChange: (event) => setOrigin(event.target.value),
          onBlur: () => {
            void persistOrigin().catch((err) => setError(err instanceof Error ? err.message : String(err)));
          }
        }
      )
    ] }),
    /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("div", { style: rowStyle, children: [
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)(
        "span",
        {
          "aria-hidden": "true",
          style: {
            width: 8,
            height: 8,
            borderRadius: 999,
            background: loggedIn ? "#22c55e" : "#9ca3af",
            display: "inline-block"
          }
        }
      ),
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)("span", { children: loggedIn ? t("loggedIn", "\u5DF2\u767B\u5F55 \xB7 OAuth \u51ED\u636E\u53EF\u7528") : t("loggedOut", "\u672A\u767B\u5F55") })
    ] }),
    /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("div", { style: rowStyle, children: [
      loggedIn ? /* @__PURE__ */ (0, import_jsx_runtime.jsx)("button", { type: "button", style: btnStyle, disabled: busy, onClick: () => {
        void onLogout();
      }, children: t("logout", "\u9000\u51FA\u767B\u5F55") }) : /* @__PURE__ */ (0, import_jsx_runtime.jsx)("button", { type: "button", style: btnStyle, disabled: busy, onClick: () => {
        void onLogin();
      }, children: busy ? t("saving", "\u4FDD\u5B58\u4E2D\u2026") : t("login", "\u767B\u5F55") }),
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)("button", { type: "button", style: btnStyle, disabled: busy, onClick: () => {
        void onImport();
      }, children: t("importLocal", "\u5BFC\u5165\u672C\u673A CLI \u51ED\u636E") })
    ] }),
    importText !== void 0 ? /* @__PURE__ */ (0, import_jsx_runtime.jsx)("pre", { style: { ...descStyle, whiteSpace: "pre-wrap" }, children: importText }) : void 0,
    (openUrl !== void 0 || userCode !== void 0) && !loggedIn ? /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("div", { style: { ...fieldStyle, gap: 8 }, children: [
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)("p", { style: { ...descStyle, margin: 0 }, children: t("waiting", "\u8BF7\u5728\u6D4F\u89C8\u5668\u4E2D\u5B8C\u6210 AI-GateWay \u767B\u5F55\u3002") }),
      userCode !== void 0 && userCode.length > 0 ? /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("div", { style: rowStyle, children: [
        /* @__PURE__ */ (0, import_jsx_runtime.jsx)("span", { children: t("userCode", "\u7528\u6237\u7801") }),
        /* @__PURE__ */ (0, import_jsx_runtime.jsx)("code", { style: codeStyle, children: userCode })
      ] }) : void 0,
      openUrl !== void 0 && openUrl.length > 0 ? /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("div", { style: fieldStyle, children: [
        /* @__PURE__ */ (0, import_jsx_runtime.jsx)("span", { children: t("openUrl", "\u9A8C\u8BC1\u5730\u5740") }),
        /* @__PURE__ */ (0, import_jsx_runtime.jsx)("a", { href: openUrl, target: "_blank", rel: "noreferrer", children: openUrl })
      ] }) : void 0
    ] }) : void 0,
    error !== void 0 || watchError !== void 0 ? /* @__PURE__ */ (0, import_jsx_runtime.jsx)("p", { style: { ...descStyle, color: "#ef4444" }, children: error ?? watchError ?? t("error", "\u51FA\u9519\u4E86") }) : void 0
  ] });
}

// src/client/locales.ts
var en = {
  nav: "AGW Oauth",
  title: "AGW Oauth",
  description: "Connect to the AI-GateWay and sign in securely in the browser.",
  originLabel: "Gateway URL",
  originPlaceholder: "https://gw.example.com",
  loggedIn: "Signed in \xB7 OAuth credentials available",
  loggedOut: "Not signed in",
  login: "Sign in",
  importLocal: "Import local CLI files",
  importHint: "Reads ~/.codex/auth.json and ~/.claude/.credentials.json. Prefer this over a new browser login.",
  logout: "Sign out",
  saving: "Saving\u2026",
  waiting: "Finish signing in to AI-GateWay in the browser.",
  userCode: "User code",
  openUrl: "Verification URL",
  error: "Something went wrong"
};
var zh = {
  nav: "AGW Oauth",
  title: "AGW Oauth",
  description: "\u8FDE\u63A5 AI-GateWay \u7F51\u5173\u5E76\u901A\u8FC7\u6D4F\u89C8\u5668\u5B89\u5168\u767B\u5F55\u3002",
  originLabel: "\u7F51\u5173\u5730\u5740",
  originPlaceholder: "https://gw.example.com",
  loggedIn: "\u5DF2\u767B\u5F55 \xB7 OAuth \u51ED\u636E\u53EF\u7528",
  loggedOut: "\u672A\u767B\u5F55",
  login: "\u767B\u5F55",
  importLocal: "\u5BFC\u5165\u672C\u673A CLI \u51ED\u636E",
  importHint: "\u8BFB\u53D6 ~/.codex/auth.json \u4E0E ~/.claude/.credentials.json\u3002\u5DF2\u6709\u6587\u4EF6\u65F6\u4E0D\u5FC5\u518D\u8D70\u6D4F\u89C8\u5668\u767B\u5F55\u3002",
  logout: "\u9000\u51FA\u767B\u5F55",
  saving: "\u4FDD\u5B58\u4E2D\u2026",
  waiting: "\u8BF7\u5728\u6D4F\u89C8\u5668\u4E2D\u5B8C\u6210 AI-GateWay \u767B\u5F55\u3002",
  userCode: "\u7528\u6237\u7801",
  openUrl: "\u9A8C\u8BC1\u5730\u5740",
  error: "\u51FA\u9519\u4E86"
};
var NS = "settings.agw-oauth";

// src/client/index.ts
var inject = ["slots", "locale"];
function apply(ctx) {
  ctx.effect(() => ctx.locale.register(NS, { zh, en }), "agw-oauth: locales");
  const t = ctx.locale.bind(NS);
  const inject2 = () => ({ t });
  ctx.slots.inject("settings.section", () => ctx.slots.register({
    name: "settings.section",
    id: "agw-oauth",
    order: 50,
    label: () => t("nav"),
    inject: inject2
  }, AgwSection));
}
return module.exports; } });
