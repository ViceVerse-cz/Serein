#!/bin/bash
# Offline regression: all Apple/keychain commands below are synthetic stubs.
set -euo pipefail

script=${1:-packaging/macos/sign-release.sh}
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
export RUNNER_TEMP="$fixture/runner temp" VERSION=1.2.3-nightly.1.1
export GITHUB_RUN_NUMBER=1 GITHUB_RUN_ATTEMPT=1
export MACOS_CERTIFICATE_BASE64=c3ludGhldGlj MACOS_CERTIFICATE_PASSWORD=synthetic-password
export MACOS_SIGNING_IDENTITY='Developer ID Application: Synthetic Owner (TESTTEAM01)'
export APPLE_ID=synthetic@example.invalid APPLE_TEAM_ID=TESTTEAM01
export APPLE_APP_SPECIFIC_PASSWORD=synthetic-apple-password
expected_fingerprint=0123456789ABCDEF0123456789ABCDEF01234567
mkdir -p "$RUNNER_TEMP" "$fixture/Serein.app/Contents/MacOS"
mkdir -p "$fixture/Serein.app/Contents/Frameworks"
for name in libavcodec-serein.61.dylib libavutil-serein.59.dylib libopenh264.8.dylib; do
  touch "$fixture/Serein.app/Contents/Frameworks/$name"
done

security() {
  case "$1" in
    list-keychains)
      if [[ "${4:-}" == -s ]]; then
        : > "$fixture/search-list"
        for entry in "${@:5}"; do printf '%s\n' "$entry" >> "$fixture/search-list"; done
      else
        sed 's/^/    "/; s/$/"/' "$fixture/search-list"
      fi ;;
    create-keychain) touch "$keychain" ;;
    set-keychain-settings|unlock-keychain|set-key-partition-list) ;;
    import) [[ "$scenario" != import-failure ]] ;;
    find-identity)
      [[ "$*" == "find-identity -v -p codesigning $keychain" ]] || return 1
      grep -Fxq "$keychain" "$fixture/search-list" || return 1
      printf '%s\n' preflight >> "$fixture/events"
      case "$scenario" in
        missing) echo '     0 valid identities found' ;;
        mismatch) printf '  1) %s "Developer ID Application: Another Owner (TESTTEAM01)"\n' "$expected_fingerprint" ;;
        ambiguous)
          printf '  1) %s "%s"\n' "$expected_fingerprint" "$MACOS_SIGNING_IDENTITY"
          printf '  2) %s "%s"\n' 89ABCDEF0123456789ABCDEF0123456789ABCDEF "$MACOS_SIGNING_IDENTITY" ;;
        *) printf '  1) %s "%s"\n     1 valid identities found\n' "$expected_fingerprint" "$MACOS_SIGNING_IDENTITY" ;;
      esac ;;
    delete-keychain) printf '%s\n' cleanup >> "$fixture/events" ;;
    *) echo "Unexpected security command: $1" >&2; return 99 ;;
  esac
}
openssl() { printf '%s\n' synthetic-keychain-password; }
function /usr/libexec/PlistBuddy() { printf '%s\n' plist >> "$fixture/events"; }
codesign() {
  printf '%s\n' codesign >> "$fixture/events"
  if [[ "$1" == --force ]]; then
    printf '%s\n' "signed ${*: -1}" >> "$fixture/events"
    if [[ "${*: -1}" == *.dylib ]]; then
      [[ " $* " != *" --entitlements "* ]] || return 1
    fi
    grep -Fxq preflight "$fixture/events" || return 1
    grep -Fxq "$keychain" "$fixture/search-list" || return 1
    [[ " $* " == *" --sign $expected_fingerprint "* ]] || return 1
    [[ " $* " == *" --keychain $keychain "* ]] || return 1
    [[ "$scenario" != signing-failure ]]
  fi
}
ditto() { :; }
xcrun() {
  printf '%s\n' "$1 $2" >> "$fixture/events"
  [[ "$scenario" != notarization-failure || "$1 $2" != 'notarytool submit' ]]
}
spctl() { printf '%s\n' verified >> "$fixture/events"; }

for scenario in success empty-search-list missing mismatch ambiguous import-failure signing-failure notarization-failure; do
  : > "$fixture/original"
  if [[ "$scenario" != empty-search-list ]]; then
    printf '%s\n' '/tmp/login keychain-db' '/tmp/other.keychain-db' > "$fixture/original"
  fi
  cp "$fixture/original" "$fixture/search-list"
  : > "$fixture/events"
  # Launch a subshell normally: an `if source ...` would disable the script's errexit.
  set +e
  ( source "$script" "$fixture/Serein.app" ) > "$fixture/output" 2>&1
  result=$?
  set -e
  cmp "$fixture/original" "$fixture/search-list"
  grep -Fxq cleanup "$fixture/events"
  [[ -z "$(ls -A "$RUNNER_TEMP")" ]]
  ! grep -Fq "$MACOS_SIGNING_IDENTITY" "$fixture/output" || exit 1
  ! grep -Fq "$MACOS_CERTIFICATE_PASSWORD" "$fixture/output" || exit 1
  ! grep -Fq "$APPLE_APP_SPECIFIC_PASSWORD" "$fixture/output" || exit 1
  if [[ "$scenario" == success || "$scenario" == empty-search-list ]]; then
    [[ "$result" == 0 ]] || { cat "$fixture/output"; echo 'Synthetic signing failed' >&2; exit 1; }
    grep -Fxq verified "$fixture/events"
    signed=$(grep '^signed ' "$fixture/events")
    expected=$(printf 'signed %s\n' "$fixture/Serein.app/Contents/Frameworks/libavcodec-serein.61.dylib" \
      "$fixture/Serein.app/Contents/Frameworks/libavutil-serein.59.dylib" \
      "$fixture/Serein.app/Contents/Frameworks/libopenh264.8.dylib" "$fixture/Serein.app")
    [[ "$signed" == "$expected" ]]
  else
    [[ "$result" != 0 ]]
    ! grep -Fxq verified "$fixture/events" || exit 1
    case "$scenario" in
      missing|mismatch|ambiguous)
        grep -Fq 'MACOS_SIGNING_IDENTITY' "$fixture/output"
        ! grep -Fxq plist "$fixture/events" || exit 1
        ! grep -Fxq codesign "$fixture/events" || exit 1
        ! grep -Fq notarytool "$fixture/events" || exit 1 ;;
    esac
  fi
done
echo 'Mac signing regression passed: exact identity, failure gates and keychain cleanup (synthetic).'
