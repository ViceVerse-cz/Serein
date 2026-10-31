# Camera continuity during Gateway resume

Synthetic offline evidence for PR #844. The images are native eframe/Wgpu framebuffers
from Serein's call UI, captured without a Discord connection or camera/audio devices.

## Revisions and isolation

- Before: PR base `1b2f8942910436f9a488a0cf0f806776f02a074f` plus the evidence harness.
- After: PR head `86b6a38de603a00db008703b9497e60e5679dad2` plus the identical harness.
- Both use Rust 1.98.1, `--features demo`, and separate source checkouts and Cargo target
  directories. The recorded target directories were `target/pr844-before` and
  `target/pr844-after`; neither was shared between checkouts.
- Build settings: `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`,
  `CARGO_BUILD_JOBS=2`. Debug assertions remain enabled.
- Capture: CachyOS x86_64, dark theme, 1040 x 720 framebuffer, 1.0 pixels per point.

The common harness also supplies the missing `managed: false` fields in four existing
role demo constructors so both revisions compile with `--features demo`. It leaves the
baseline camera predicate unchanged.

## Production path exercised

The replay begins with `test_support::call_demo_state()`, an established call, camera
requested, and a nonzero synthetic request ID. It applies the real `Event::Disconnected`
through `State::apply`, then evaluates the production `camera_preview_allowed` predicate.
If eligibility is lost, it obtains the actual `SetCamera(false)` command from State,
runs the production `voice_offline_failure` admission check, and reduces its failure event.
The final phase is derived from these operations, not assigned by the capture fixture.
The existing offline admission branch was extracted into that shared pure check without
changing its condition or failure event; both revisions use the same extraction.

The replay restores demo mode before rendering. Its dedicated native app has no service,
credential-store, capture, encoding or playback adapters, ignores UI commands, and clears
input events during capture. It saves only its own framebuffer and closes afterward.

| Result after Gateway disconnect | Before | After |
| --- | --- | --- |
| Camera preview eligible | false | true |
| Camera-off command emitted | true | false |
| Final call phase | Failed | Connected |
| Camera still requested | false | true |
| New camera activation eligible | false | false |

This isolates state, permission and command-admission behavior. It does not execute the
physical camera worker or establish actual Discord reconnection, video or audio continuity.
The after image is an offline preview of a connected model, not a live call.

## Reproduce

Start from the evidence commit referenced by the PR image URLs. Create a separate checkout
at the base revision, then apply only the common evidence changes to it:

```sh
git diff --binary 86b6a38de603a00db008703b9497e60e5679dad2 HEAD -- \
  apps/desktop/src/main.rs apps/desktop/src/voice.rs \
  apps/desktop/src/connection.rs apps/desktop/src/post_menu_demo.rs \
  apps/desktop/src/camera_gateway_demo.rs > /tmp/pr844-harness.patch
git -C /path/to/base-checkout apply /tmp/pr844-harness.patch
```

Use Rust 1.98.1 and the build settings above. In each checkout, set `CARGO_TARGET_DIR`
to its own directory, then run the same commands:

```sh
cargo test --locked --offline -p serein --features demo --bin serein \
  camera_gateway_demo::tests::disconnected_gateway_retains_active_camera_but_blocks_new_activation \
  -- --exact --nocapture
cargo build --locked --offline -p serein --features demo --bin serein
"$CARGO_TARGET_DIR/debug/serein" --demo --demo-check-camera-gateway
"$CARGO_TARGET_DIR/debug/serein" --demo \
  --demo-camera-gateway-evidence=/absolute/path/to/capture.png
```

The base regression/check intentionally fails; its native capture still succeeds and
renders that derived failure. The fixed regression/check passes. Both evidence modes
refuse to start without `--demo`; an empty output path is rejected before normal startup.

Observed regression output:

```text
Before: assertion `left == right` failed
  left: Some(Failed)
 right: Some(Connected)
test result: FAILED. 0 passed; 1 failed

After: Offline camera Gateway replay passed: active preview retained; new activation remains unavailable.
test result: ok. 1 passed; 0 failed
```

## Captured files

Both files were visually inspected. They are direct PNG encodings of the native
framebuffer, with no cropping, resizing or substituted pixels.

| File | Bytes | Dimensions |
| --- | ---: | --- |
| `before.png` | 363,108 | 1040 x 720 |
| `after.png` | 367,155 | 1040 x 720 |

SHA-256:

```text
before.png  96d068f1bfc1010145f2ddffc0882a22714a397e46cb003f40655a6d03cb144b
after.png   f02560f71ff49ee7202ad9e03bb4fefaafb95038c4cff717deeee0a09253bc88
```

The shared `camera_gateway_demo.rs` source SHA-256 at capture time was
`c83de046b2e01fb08a4e1fa4a429f3f7fd41cbbe3ac2ec28d04a18e536d0cbc8`.
