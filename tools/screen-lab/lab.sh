#!/usr/bin/env bash
# Screen-share lab for a disposable headless wlroots compositor (for example labwc with the
# headless backend behind wayvnc). Starts an unattended ScreenCast portal, plays synthetic
# desktop scenes and runs the sender bench against them. Nothing joins Discord or records.
#
#   tools/screen-lab/lab.sh portal            # (re)start xdg-desktop-portal-wlr for the lab
#   tools/screen-lab/lab.sh scene text        # idle | text | motion | stress
#   tools/screen-lab/lab.sh bench --size 1080 --fps 30 --seconds 15
#   tools/screen-lab/lab.sh matrix [BENCH...] # scenes x presets, one line per run
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../.." && pwd)
export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
export WAYLAND_DISPLAY=${WAYLAND_DISPLAY:-wayland-0}
export XDG_CURRENT_DESKTOP=${XDG_CURRENT_DESKTOP:-wlroots}
state="$XDG_RUNTIME_DIR/serein-screen-lab"
mkdir -p "$state"
bench_bin=${BENCH:-$root/target/release/examples/screen_bench}

portal() {
	[[ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]] || {
		echo "No Wayland compositor at $XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" >&2
		exit 1
	}
	# The lab config picks the first output without a chooser; -r replaces a running one.
	setsid /usr/libexec/xdg-desktop-portal-wlr -r -l WARN -c "$here/xdpw.conf" \
		>"$state/xdpw.log" 2>&1 </dev/null &
	sleep 0.5
	if ! pgrep -f '^/usr/libexec/xdg-desktop-portal( |$)' >/dev/null; then
		setsid /usr/libexec/xdg-desktop-portal -r >"$state/xdp.log" 2>&1 </dev/null &
		sleep 1
	fi
	busctl --user get-property org.freedesktop.portal.Desktop /org/freedesktop/portal/desktop \
		org.freedesktop.portal.ScreenCast version >/dev/null
}

scene() {
	if [[ -f "$state/scene.pid" ]]; then
		kill -- "-$(cat "$state/scene.pid")" 2>/dev/null || true
		rm -f "$state/scene.pid"
		sleep 0.3
	fi
	local command
	case "${1:-idle}" in
	idle) return ;;
	# Colored source code scrolling at roughly 200 lines per second: text-heavy and sharp.
	text) command=(foot --fullscreen sh -c "while :; do bat --color=always --paging=never --style=numbers $root/crates/discord-voice/src/*.rs | while IFS= read -r line; do printf '%s\n' \"\$line\"; sleep 0.005; done; done") ;;
	# A moving object over a mostly static picture, like a cursor or small animation.
	motion) command=(gst-launch-1.0 -q videotestsrc pattern=ball is-live=true ! video/x-raw,width=1920,height=1080,framerate=60/1 ! timeoverlay font-desc="Sans 36" ! waylandsink fullscreen=true) ;;
	# Whole-screen motion with a noise block: beyond any bitrate, shows rate-control skips.
	stress) command=(gst-launch-1.0 -q videotestsrc pattern=smpte horizontal-speed=6 is-live=true ! video/x-raw,width=1920,height=1080,framerate=60/1 ! timeoverlay font-desc="Sans 48" ! waylandsink fullscreen=true) ;;
	*)
		echo "unknown scene $1" >&2
		exit 2
		;;
	esac
	setsid "${command[@]}" >"$state/scene.log" 2>&1 </dev/null &
	echo $! >"$state/scene.pid"
	sleep 1.5
}

# xdg-desktop-portal-wlr 0.8 sometimes never schedules a new consumer after its start-up
# renegotiation (it logs "out of buffers" and the source only repeats its keepalive), and can
# exit on a protocol error. Both also happen with the unchanged pipeline, so the backend is
# restarted for every attempt and a moving scene that never got going is retried.
bench() {
	local attempt output fps
	for attempt in 1 2 3 4 5; do
		portal
		output=$("$bench_bin" "$@" 2>&1) || true
		fps=$(sed -n 's/^frames: [0-9]* (\([0-9]*\).*/\1/p' <<<"$output")
		if [[ "${LAB_SCENE:-}" != idle ]] && ((${fps:-0} < 3)); then
			echo "[lab] attempt $attempt stalled in the portal; retrying" >&2
			continue
		fi
		break
	done
	printf '%s\n' "$output"
}

matrix() {
	local bins=("$@") scenes=(idle text motion stress)
	[[ ${#bins[@]} -gt 0 ]] || bins=("$bench_bin")
	printf '%-8s %-6s %-10s %8s %8s %8s %10s %8s\n' scene preset build fps cpu% kbps gap_p95ms rss_mb
	for name in "${scenes[@]}"; do
		scene "$name"
		export LAB_SCENE=$name
		for preset in "720 30" "1080 30" "1080 60"; do
			set -- $preset
			for bin in "${bins[@]}"; do
				local out
				out=$(BENCH=$bin bench --seconds "${SECONDS_PER_RUN:-12}" --size "$1" --fps "$2" --no-preview)
				printf '%-8s %-6s %-10s %8s %8s %8s %10s %8s\n' "$name" "$1p$2" "$(basename "$(dirname "$(dirname "$(dirname "$bin")")")")" \
					"$(sed -n 's/^frames: [0-9]* (\([0-9.]*\) fps.*/\1/p' <<<"$out")" \
					"$(sed -n 's/^cpu: \([0-9.]*\)%.*/\1/p' <<<"$out")" \
					"$(sed -n 's/^bitrate: \([0-9]*\) kbps.*/\1/p' <<<"$out")" \
					"$(sed -n 's/^frame gap ms: .*p95 \([0-9.]*\).*/\1/p' <<<"$out")" \
					"$(sed -n 's/^rss: .*, \([0-9]*\) MiB peak/\1/p' <<<"$out")"
			done
		done
	done
	scene idle
}

case "${1:-}" in
portal) portal ;;
scene)
	scene "${2:-idle}"
	echo "${2:-idle}" >"$state/scene.name"
	;;
bench)
	shift
	LAB_SCENE=$(cat "$state/scene.name" 2>/dev/null || echo idle) SEREIN_VOICE_DIAGNOSTICS=${SEREIN_VOICE_DIAGNOSTICS:-1} bench "$@"
	;;
matrix)
	shift
	SEREIN_VOICE_DIAGNOSTICS=1 matrix "$@"
	;;
*)
	sed -n '2,10p' "$0"
	exit 2
	;;
esac
