# Historical AMD quality / responsive UI evidence

Historical evidence from the original combined PR, separated by scope. The recorded revision labels, hashes and measurements are unchanged; these results do not validate the new branch heads.

Baseline: `be644d9064e0b53caf6db103037c8a48a5c7baa8`. The original synthetic native eframe/wgpu captures use the same fonts, viewport and zoom before/after. They are isolated development previews, not the shipped desktop app.

`settings-*`, `friends-*` and `reply-*` show responsive settings, Add Friend and long-name reply layouts. [prepare-preview.py](prepare-preview.py) and [sample-native.py](sample-native.py) reproduce the original historical isolated preview when given its recorded checkout; its fixture flags are not production CLI options.

The reply fixture sampled one run per revision after three seconds warmup, then 15 one-second samples. CPU was below timer resolution and the -905,216-byte RSS difference is noise. Raw samples/source hashes are in [measurements.json](measurements.json); no shipped-package improvement is claimed.
