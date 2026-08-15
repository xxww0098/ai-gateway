/**
 * TypeScript 7 does not yet ship the classic programmatic API that
 * typescript-eslint needs. Redirect require("typescript") to the TS 6
 * compatibility package for the eslint process only.
 *
 * See: https://devblogs.microsoft.com/typescript/announcing-typescript-7-0/
 */
const Module = require("module");
const path = require("path");

const originalResolve = Module._resolveFilename;

function resolveTs6(parent, isMain, options) {
  const candidates = [
    // Installed via @typescript/typescript6 → @typescript/old (npm:typescript@6)
    path.join(__dirname, "../node_modules/@typescript/old"),
    "@typescript/old",
  ];
  for (const candidate of candidates) {
    try {
      return originalResolve.call(Module, candidate, parent, isMain, options);
    } catch {
      // try next
    }
  }
  return null;
}

Module._resolveFilename = function (request, parent, isMain, options) {
  if (request === "typescript") {
    const resolved = resolveTs6(parent, isMain, options);
    if (resolved) return resolved;
  }
  return originalResolve.call(this, request, parent, isMain, options);
};
