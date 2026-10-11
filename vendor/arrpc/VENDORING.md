# Vendored `arrpc` (Rich Presence server)

## Provenance

- Upstream project: [OpenAsar/arrpc](https://github.com/OpenAsar/arrpc),
  `arrpc@3.5.0`, MIT licensed, copyright (c) 2022 OpenAsar
  (see `LICENSE` in this directory — preserved verbatim).
- This copy is byte-identical to the previously pinned Git-hosted `arrpc`
  specification (commit `efe7589762470d32b9ba10d529be5acc23cd0e19`,
  runtime subset: `src/`, `LICENSE`, `README.md`, `changelog.md`). Only host-specific files were omitted (`.github/`,
  `.vscode/`, `examples/`, `ext/`, `update_db.js`, lockfiles, `.bun-tag`,
  `.gitignore`, `.prettierignore`, the `packageManager` pin, and the `bin`
  launcher — none of which Remcord imports).
- No source file in this directory was modified; `package.json` metadata was
  rewritten only to drop workspace-irrelevant host fields. The upstream
  `repository`/`homepage`/`bugs` attributions are kept intact.

## Why vendored

Remcord has no remote dependency home of its own, and no network fetch of a
replacement upstream could be verified from this environment. Vendoring the
exact pinned runtime tree keeps the Discord Rich Presence protocol behavior
bit-for-bit identical while removing every remote URL that referenced the
previous product identity from `package.json` and all lockfiles.

Consumers: `src/rpc.ts` (worker) and `src/discord/rpcProcess.ts` import the
`arrpc` package specifier, which now resolves to this directory via the
`file:vendor/arrpc` dependency in the root `package.json`. The public API
surface is unchanged (`RPCServer` default export, `GameList`/`ServerSettings`
types, `getProcessesList()`).
