# Nix package

The flake exposes `serein` (also the default package) and a development shell for
`x86_64-linux` and `aarch64-darwin`. Build on the matching host:

```sh
nix build .#serein
nix run .#serein
nix develop        # cargo, rustc and the native build inputs
```

Linux installs `result/bin/serein`, a `cz.viceverse.serein.desktop` entry and icons.
macOS installs `result/Applications/Serein.app` with `result/bin/serein` linking to
the bundle executable. Both include the notices, font/asset licenses and the
corresponding MPL-2.0 hpke-rs source under `result/share/doc/serein` (inside the
bundle `Resources` on macOS).

The package version is the Cargo workspace version, as with `cargo build`. Nix
installations are updated through Nix; the in-app updater does not replace store
paths. The macOS bundle is not Developer ID signed or notarized.

CI builds both systems and compares installed notices and source with the checkout.
A successful build does not establish live Discord, keyring, screen capture or
physical audio compatibility. Linux still needs a graphical session, GPU driver,
Secret Service provider and an XDG desktop portal backend.

## Optional public binary cache

The Nix workflow supports a public Cachix cache, but no project cache is configured
yet. Maintainers must create a cache in [Cachix](https://docs.cachix.org/getting-started),
set the repository variable `CACHIX_CACHE_NAME` to its name, and store a cache-scoped
write token in the repository secret `CACHIX_AUTH_TOKEN`. No token belongs in the
flake or in user configuration. Leaving the variable unset preserves the current
build workflow. A configured cache without a token is read-only.

Only pushes to `main` can upload the built `result` closure, including dependencies,
after the job's package checks pass. Pull requests and manual runs may substitute
from the public cache but receive no Cachix write token and never upload results.
The workflow covers both supported systems; cache hits require matching flake inputs
and source derivations, so changing source can still require a local build.

After maintainers publish the cache name and seed it with a successful `main` build,
users can enable it explicitly:

```sh
cachix use <published-cache-name>
nix build github:ViceVerse-cz/Serein#serein
```

`cachix use` configures the substituter and its public signing key. See the
[Cachix setup guide](https://docs.cachix.org/getting-started) for multi-user Nix
permissions. Verify cache substitution on a fresh machine before treating the cache
as available; this repository change alone does not create or seed a cache.
