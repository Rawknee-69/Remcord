import assert from "node:assert/strict";
import { existsSync, readFileSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const vendorRoot = join(repoRoot, "vendor/arrpc");

test("vendored arRPC keeps its product-neutral identity and license", () => {
    const manifest = JSON.parse(readFileSync(join(vendorRoot, "package.json"), "utf-8"));
    assert.equal(manifest.name, "arrpc");
    assert.equal(manifest.version, "3.5.0");
    assert.equal(manifest.license, "MIT");
    assert.equal(manifest.author, "OpenAsar");
    assert.match(manifest.repository.url, /OpenAsar\/arrpc/);
    assert.equal(JSON.stringify(manifest).toLowerCase().includes("legcord"), false);
    const license = readFileSync(join(vendorRoot, "LICENSE"), "utf-8");
    assert.match(license, /Copyright \(c\) 2022 OpenAsar/);
    assert.match(license, /MIT License/);
});

test("vendored arRPC runtime tree is complete (protocol behavior preserved)", () => {
    for (const file of [
        "src/server.js",
        "src/bridge.js",
        "src/index.js",
        "src/transports/ipc.js",
        "src/transports/websocket.js",
        "src/process/index.js",
        "src/process/detectables.json",
        "src/process/native/index.js",
        "src/process/native/win32.js",
        "src/process/native/win32Legacy.js",
        "src/process/native/linux.js",
        "src/process/native/darwin.js",
        "src/@types/server.d.ts",
    ]) {
        assert.equal(existsSync(join(vendorRoot, file)), true, `missing ${file}`);
    }
    assert.ok(statSync(join(vendorRoot, "src/process/detectables.json")).size > 1_000_000);
});

test("vendored arRPC exposes the expected RPC protocol surface", async () => {
    const serverModule = (await import(pathToFileURL(join(vendorRoot, "src/server.js")).href)) as {
        default: new (...args: unknown[]) => { getProcessesList: unknown; on: unknown };
    };
    assert.equal(typeof serverModule.default, "function");
    assert.equal(typeof serverModule.default.prototype.getProcessesList, "function");
    const source = readFileSync(join(vendorRoot, "src/server.js"), "utf-8");
    for (const token of ["DISPATCH", "SET_ACTIVITY", "INVITE_BROWSER", "getProcessesList"]) {
        assert.match(source, new RegExp(token));
    }
    const types = readFileSync(join(vendorRoot, "src/@types/server.d.ts"), "utf-8");
    assert.match(types, /ServerSettings/);
    assert.match(types, /getProcessesList/);
});

test("workspace resolves arRPC locally with no remote URL", () => {
    const root = JSON.parse(readFileSync(join(repoRoot, "package.json"), "utf-8"));
    assert.equal(root.dependencies.arrpc, "file:vendor/arrpc");
    for (const lockfile of ["pnpm-lock.yaml", "bun.lock"]) {
        const lock = readFileSync(join(repoRoot, lockfile), "utf-8");
        assert.doesNotMatch(lock, /legcord/i);
    }
    const rpcWorker = readFileSync(join(repoRoot, "src/rpc.ts"), "utf-8");
    assert.match(rpcWorker, /from "arrpc"/);
    const rpcProcess = readFileSync(join(repoRoot, "src/discord/rpcProcess.ts"), "utf-8");
    assert.match(rpcProcess, /from "arrpc"/);
});

test("About screen makes no obsolete contributor-endpoint request", () => {
    const source = readFileSync(join(repoRoot, "src/shelter/settings/components/AboutPopup.tsx"), "utf-8");
    assert.doesNotMatch(source, /legcord/i);
    assert.doesNotMatch(source, /api\.github\.com/);
    assert.doesNotMatch(source, /fetch\(/);
    assert.match(source, /Contributor information is not configured for this build/);
});

test("packaging workflow trusts no foreign signing certificate", () => {
    const source = readFileSync(join(repoRoot, ".github/workflows/package.yml"), "utf-8");
    assert.doesNotMatch(source, /legcord/i);
    assert.doesNotMatch(source, /NewSign/);
    assert.match(source, /MACOS_CSC_LINK/);
});
