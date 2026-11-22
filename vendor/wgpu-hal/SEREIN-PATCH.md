# Windows Eco adapter probe

The vendored wgpu-hal and wgpu-types 30.0.1 come from upstream commit
40f4a34ebaf56f9a046231f54125ad046239d3f3 and retain their original manifests and
licenses. The crates.io archive SHA-256 values follow.

- wgpu-hal: b6b7fb58561a792bc237628ba0792e332de418fefe145f13b5ed8201e6d52f58
- wgpu-types: 99dad6f1fbdbbdb4c278a6508b059d44688f5cebddf78d005a46a31340269286

The default DX12 path probes every adapter by creating a device on each.
The opt-in instance option uses DXGI minimum-power ordering to expose just the
first eligible hardware adapter. An unavailable probe returns no adapter, so
the caller can retry with the ordinary backend and adapter policy.

Remove both patches when wgpu can request a low-power DX12 adapter before
probing the remaining devices.
