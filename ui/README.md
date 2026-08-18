# Lynx Launcher UI

ReactLynx and TypeScript UI built with Rspeedy. The production artifact is
`dist/main.lynx.bundle`.

## Toolchain

- Node.js `^20.19.0 || >=22.12.0`
- pnpm `10.34.5` (pinned by `packageManager`)
- Dependencies are pinned to the versions used by the current
  `create-rspeedy@0.16.4` React TypeScript template family.

Install and verify:

```sh
pnpm install --frozen-lockfile
pnpm test
pnpm typecheck
pnpm build
```

## Native contract

The desktop host registers a native module named `Launcher` with
`lynx_view_builder_register_native_module`. Its N-API methods create and return
real JavaScript promises:

```ts
interface Launcher {
  getApplications(): Promise<Array<{
    id: string
    name: string
    iconUri?: string | null
  }>>
  launchApplication(id: string): Promise<void>
}
```

`getApplications()` resolves to an array. The UI boundary rejects a non-array
result, non-object entries, blank or non-string IDs and names, unsupported
`iconUri` values, and duplicate IDs. Duplicate IDs are rejected rather than
silently merged because they would make rendering keys and launch targets
ambiguous.

`launchApplication(id)` resolves with `undefined` after the host accepts the
launch, and rejects with the platform error on failure.

`iconUri` is passed directly to the Lynx `<image>` element. Missing, blank, or
failed image URIs fall back to a deterministic colored initial.

The UI uses Lynx elements and events only. It does not rely on DOM or browser
globals. The desktop host must include the Lynx `<input>` XElement for search.
