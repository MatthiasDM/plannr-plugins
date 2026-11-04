# Plannr plugins

Installable plugins for [Plannr](https://github.com/MatthiasDM/plannr-modeller), the SketchUp-style web
modeller. Each plugin is one `.plannr-plugin` file: download it, install it in Plannr, and switch it on
or off whenever you like.

| Plugin | What it does | Download |
|---|---|---|
| **[3D Print](print3d/README.md)** | Exports the selection or the whole model as **3MF** or **STL** for your slicer (Bambu Studio, PrusaSlicer, OrcaSlicer, Cura): millimetres, Z up, any scale, welded and repaired into watertight solids, checked against your printer's build volume and laid out on the plate. | [plannr-plugin-print3d-0.1.0](https://github.com/MatthiasDM/plannr-plugins/releases/download/print3d-v0.1.0/plannr-plugin-print3d-0.1.0.plannr-plugin) |
| **[Car Track](track/README.md)** | Designs printable track for 1:64 die-cast cars (the orange Hot Wheels-style channel): straights, curves, ramps and loops that chain end to end, connectors that clip pieces together, supports, *Split to fit* for your printer, and a *Test drive* that runs a car over the track to find the launch speed it needs. Works with 3D Print to download the pieces. | [plannr-plugin-track-0.2.0](https://github.com/MatthiasDM/plannr-plugins/releases/download/track-v0.2.0/plannr-plugin-track-0.2.0.plannr-plugin) |

Every version is on the [releases page](https://github.com/MatthiasDM/plannr-plugins/releases).

## Install, switch off, remove

1. Download a `.plannr-plugin` file above.
2. In Plannr: **Plugins → Manage plugins… → Install from file**, and pick it (or drop the file on that
   dialog). The app reloads with the plugin's tools and panels.
3. The same dialog has an on/off switch for every plugin and **Remove** for installed ones. A plugin that
   is off keeps its objects in your models; they show again when it is back on.

Plugins need a Plannr server with plugin packages (Plannr 0.2.0 from
[plannr-modeller](https://github.com/MatthiasDM/plannr-modeller), November 2026 or newer). On a public
instance only the server's admin may install them.

## How a plugin is made

A plugin has an **engine** (Rust: custom entity types, ops that are also MCP tools, export formats),
compiled to WebAssembly and run by the Plannr server in a sandbox, and a **browser half** (TypeScript:
tools, panels, Entity Info). See Plannr's
[PLUGINS.md](https://github.com/MatthiasDM/plannr-modeller/blob/main/docs/PLUGINS.md) for the plugin API.

```
plannr/            Plannr itself (git submodule): plannr-core for the engines, the web types for the browser halves
<id>/plugin.json   the manifest: id, name, version, description, the oldest Plannr it runs on
<id>/engine/       the engine crate (crate-type rlib + cdylib, ends with plannr_core::wasm_plugin!(…))
<id>/web/index.ts  the browser half (window.plannr.registerPlugin({…}))
<id>/README.md     its documentation (also packed as README.md)
```

## Building

```bash
git clone --recursive https://github.com/MatthiasDM/plannr-plugins && cd plannr-plugins
rustup target add wasm32-wasip1
npm install
cargo test                      # the engines' tests, natively
npx tsc --noEmit                # type-check the browser halves against plannr/web
scripts/package.sh              # → dist/plannr-plugin-<id>-<version>.plannr-plugin
```

To publish a new version, raise `version` in `<id>/plugin.json`, commit, and run
`scripts/release.sh <id>`: it builds the package, tags `<id>-v<version>` and creates the GitHub release
with the file attached.

## License

MIT — see [LICENSE](LICENSE).
