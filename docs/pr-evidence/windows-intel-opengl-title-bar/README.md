# Windows Intel OpenGL native title bar evidence

Baseline: `fd92226e` (`upstream/main` when work began).
Candidate: `fix/windows-intel-opengl-title-bar` (same pinned Rust 1.99.1 toolchain and `Cargo.lock`).
No account, Discord connection, microphone or live media was used.

## Native synthetic screenshots

The screenshots are native Windows 1120 × 760 Serein windows captured via `PrintWindow`
(`PW_RENDERFULLCONTENT`) at 1.00 display scale using debug `--features demo` builds on an
NVIDIA GeForce GTX 1060 3GB host (DX12).

- `before.png`: untouched baseline (`fd92226e`) launched with `--demo --demo-settings=general`.
  Shows Serein's custom undecorated title strip (`Offset=(0, 0)`, `Client=1120×760`) and the
  enabled `Hide Serein title bar` switch turned off.
- `after.png`: candidate launched with `--demo --demo-settings=general --demo-native-title-bar`.
  Exercises the driver-required native title bar path offline without Intel hardware: the window
  uses the native Windows title bar (`Offset=(8, 31)`, `Client=1104×721`), hides Serein's custom
  title strip, and locks `Hide Serein title bar` on with the Intel OpenGL explanation without
  changing the saved preference.

Separately verified on the same host:

- `--demo --demo-rendering --demo-native-title-bar` (DX12): `Client: 1104 × 721 physical px`
  matched `WGPU surface: 1104 × 721 px / pixels/point: 1` with no mismatch warning.
- `--demo --demo-login --demo-native-title-bar` (DX12): sign-in header omitted custom caption
  buttons under the native Windows title bar.
- `WGPU_BACKEND=gl --demo --demo-settings=general --demo-native-title-bar`: started cleanly on
  OpenGL with `WindowRect=1120×760`, `Client=1104×721`, `Offset=(8, 31)`.
- `WGPU_BACKEND=gl --demo --demo-settings=general`: non-Intel OpenGL (NVIDIA GTX 1060) retained
  Serein's custom undecorated title bar (`Client=1120×760`, `Offset=(0, 0)`).

Physical Intel HD Graphics 4600 (Haswell / legacy Intel OpenGL ICD) validation remains pending
because this host exposes an NVIDIA GeForce GTX 1060 only.

## Reproduction

```powershell
cargo build --locked -p serein --features demo --bin serein
.\target\debug\serein.exe --demo --demo-settings=general
.\target\debug\serein.exe --demo --demo-settings=general --demo-native-title-bar
```
