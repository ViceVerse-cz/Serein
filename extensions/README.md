# Serein extensions

The extension SDK and the official theme and plugin catalog for Serein. Plugins build
against the one [SDK](sdk); the [authoring guide](../examples/extensions/README.md)
covers writing your own.

| Path | Contents |
| --- | --- |
| `sdk/` | The extension SDK every plugin builds against |
| `pack.py` | Packages a manifest and a built Wasm module |
| `themes/` | Declarative theme packages |
| `plugins/<id>/` | Plugin source and manifest ([build instructions](plugins/README.md)) |
| `plugins/packages/` | Reviewed plugin packages clients install |
| `previews/` | Catalog preview images ([provenance](previews/README.md)) |
| `catalog.json` | Generated; never edit by hand |

`catalog.json` lists each package's manifest, exact byte length, SHA-256 hash and an
immutable URL pinned to a full commit on `main`. The client reads it from `main`,
downloads package data only, and verifies every hash before installing. It never
clones this repository or runs build scripts. Capabilities and updates go through the
client's explicit consent flow.

## Publishing a package

1. Edit a theme under `themes/` or plugin source under `plugins/`. Preserve
   `manifest.id` and bump `manifest.version` for any changed package.
2. For a plugin, rebuild and repackage it under `plugins/packages/` (see
   [plugins/README.md](plugins/README.md)). The source and packaged manifests must match.
3. Optionally add a PNG preview under `previews/` with the package's file stem.
4. Open a pull request. CI validates every package, manifest and preview
   (`python3 catalog.py --validate`), builds the plugins and runs them in the host sandbox.
5. After merge, the [catalog workflow](../.github/workflows/extension-catalog.yml) runs
   `catalog.py --pin` on the merged commit and commits the regenerated catalog to `main`.
   Pull requests are squash-merged, so only a commit on `main` is a durable pin.

Do not commit `catalog.json` changes in a pull request. To check a catalog locally
against its pinned commit, run `python3 catalog.py --check`; `python3 test_catalog.py`
covers the validator itself.

Catalog tooling uses only Python's standard library. It checks identities,
capabilities/actions, byte bounds, plugin Wasm headers, source manifests, previews and
hashes. It intentionally does not duplicate Serein's theme schema or Wasm sandbox
validation: the client runs `extensions::parse_package` and sandbox checks before
installation, and CI runs the committed packages through the same code.

PNG previews share the package file stem and must fit 256 KiB. Full-size covers stay
embedded in their packages.

## History

These packages were maintained in `ViceVerse-cz/Serein-extensions` until they moved
here. That repository's catalog and package URLs remain valid for clients released
before the move. Manifest `source` links that point there are part of the reviewed
package bytes and change only with a new package version.

## Imported plugin build evidence

The two original plugins build from the locked `plugins/` workspace with Rust 1.98.1 for `wasm32-unknown-unknown`.
Emoji & Sticker Images reproduces the imported 70,629-byte Wasm exactly.
The imported Message delete protector contains 115,250 bytes of Wasm; the current source/SDK builds 70,629 bytes and does not reproduce that older artifact byte-for-byte.
The existing distributed package is intentionally preserved. A future release should review and version a rebuilt artifact together with its source; this import is not a claim of reproducibility for that older package.

These measurements used the standalone SDK copy that the plugins built against before
the move. All four plugins now build against the in-repository SDK, so rebuilt sizes may
differ; the committed packages are unchanged. The Custom Rich Presence catalog preview
is the native editor rendered with synthetic offline data, not live Discord
compatibility evidence.

## Provenance and licenses

Package bytes and stable IDs are preserved from the existing client/theme repository:

- Nine original themes, two plugin packages, their Rust source/SDK/build files, and Ocean preview originate from `ViceVerse-cz/Serein` at `9e6bbcdef06b997ee3de3e228b2f2a54c940b7d0`.
- Forest Piano and Soft White originate from the owner's desktop packages normalized for Serein import at `a217f86c56b13ee51c4eb956f235b13eafb7609f`.
- All eleven theme packages are also recorded in `ViceVerse-cz/Serein-themes` at `6d2a97be4f8b0040aa9eada5648bceff55acb612`.
- Existing authors, license labels, and source URLs remain in every manifest. Serein's MIT and Apache-2.0 notices are retained at the repository root and do not replace individual artwork terms.
- Forest Piano and Soft White retain supplied `CC0-1.0` metadata; this is not independent verification of image rights. Consult their original sources for artwork provenance.

This repository does not relicense third-party artwork or assert additional redistribution rights.
