# Windows binding compatibility

Source: crates.io `gpu-allocator` 0.28.0, upstream commit `3d4466e3331c43d1f85b937b775427b07d7a32fe`. Original registry archive SHA-256: `51255ea7cfaadb6c5f1528d43e92a82acb2b96c43365989a28b2d44ee38f8795`. Retains upstream source, manifest provenance and MIT/Apache-2.0 licenses.

Only Cargo.toml changes: Windows binding bounds are `>=0.62, <0.63` instead of `<=0.62`, which excluded 0.62 patch releases. This makes the allocator and wgpu-hal 30 use the same Windows 0.62.2 interfaces; the lower bound stops `cargo update` from re-resolving the allocator to Windows 0.61, which breaks the Direct3D backend. No allocator or rendering source changes. Workspace Cargo.lock controls resolution; the upstream crate lockfile is omitted.

Remove this patch when an upstream compatible release accepts Windows 0.62 patch releases. Windows x64 and ARM native CI builds validate the Direct3D backend.
