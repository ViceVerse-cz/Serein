#!/bin/bash
# CI only: secrets stay in the ephemeral runner keychain, never in artifacts.
set -euo pipefail
set +x

: "${RUNNER_TEMP:?}" "${VERSION:?}"
: "${GITHUB_RUN_NUMBER:?}" "${GITHUB_RUN_ATTEMPT:?}"
: "${MACOS_CERTIFICATE_BASE64:?}" "${MACOS_CERTIFICATE_PASSWORD:?}"
: "${MACOS_SIGNING_IDENTITY:?}" "${APPLE_ID:?}" "${APPLE_TEAM_ID:?}"
: "${APPLE_APP_SPECIFIC_PASSWORD:?}"
case "$MACOS_SIGNING_IDENTITY" in
  'Developer ID Application: '*) ;;
  *) echo 'A Developer ID Application identity is required' >&2; exit 1 ;;
esac

app=${1:?Expected a packaged .app path}
test -d "$app/Contents/MacOS"
original_keychains=()
keychain_list=$(security list-keychains -d user)
while IFS= read -r entry; do
  [[ "$entry" =~ \"(.*)\" ]] && original_keychains+=("${BASH_REMATCH[1]}")
done <<< "$keychain_list"
temporary=$(mktemp -d "$RUNNER_TEMP/serein-signing.XXXXXX")
keychain="$temporary/signing.keychain-db"
cleanup() {
  security list-keychains -d user -s ${original_keychains[@]+"${original_keychains[@]}"} >/dev/null 2>&1 || true
  security delete-keychain "$keychain" >/dev/null 2>&1 || true
  rm -rf "$temporary"
}
trap cleanup EXIT
umask 077
keychain_password=$(openssl rand -hex 32)
printf '::add-mask::%s\n' "$keychain_password"
printf '%s' "$MACOS_CERTIFICATE_BASE64" | base64 --decode > "$temporary/certificate.p12"
security create-keychain -p "$keychain_password" "$keychain"
security set-keychain-settings -lut 7200 "$keychain"
security unlock-keychain -p "$keychain_password" "$keychain"
security import "$temporary/certificate.p12" -k "$keychain" \
  -P "$MACOS_CERTIFICATE_PASSWORD" -T /usr/bin/codesign >/dev/null
security set-key-partition-list -S apple-tool:,apple:,codesign: \
  -s -k "$keychain_password" "$keychain" >/dev/null
rm "$temporary/certificate.p12"
# --keychain limits identity lookup; certificate-chain lookup still uses this list.
security list-keychains -d user -s "$keychain" ${original_keychains[@]+"${original_keychains[@]}"}
identities=$(security find-identity -v -p codesigning "$keychain")
signing_hashes=()
while read -r index fingerprint identity; do
  if [[ "$fingerprint" =~ ^[[:xdigit:]]{40}$ && "$identity" == "\"$MACOS_SIGNING_IDENTITY\"" ]]; then
    signing_hashes+=("$fingerprint")
  fi
done <<< "$identities"
if [[ "${#signing_hashes[@]}" != 1 ]]; then
  echo 'Expected exactly one valid identity matching MACOS_SIGNING_IDENTITY in the imported keychain.' >&2
  echo 'Check the full Developer ID Application name and export its certificate AND private key as a .p12; also check certificate expiry and trust chain.' >&2
  exit 1
fi

/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString ${VERSION%%-*}" "$app/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Add :CFBundleVersion string $GITHUB_RUN_NUMBER.$GITHUB_RUN_ATTEMPT" "$app/Contents/Info.plist"
sign_options=(--force --timestamp --options runtime --keychain "$keychain" --sign "${signing_hashes[0]}")
if [[ -d "$app/Contents/Frameworks" ]]; then
  for library in "$app/Contents/Frameworks/"*.dylib; do
    [[ -f "$library" ]] || continue
    codesign "${sign_options[@]}" "$library"
  done
fi
sign_options+=(--entitlements packaging/macos/voice.entitlements)
codesign "${sign_options[@]}" "$app"
codesign --verify --deep --strict --verbose=2 "$app"

xcrun notarytool store-credentials serein-release --keychain "$keychain" \
  --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" \
  --password "$APPLE_APP_SPECIFIC_PASSWORD" >/dev/null
ditto -c -k --keepParent "$app" "$temporary/notarization.zip"
xcrun notarytool submit "$temporary/notarization.zip" \
  --keychain "$keychain" --keychain-profile serein-release --wait --timeout 30m
xcrun stapler staple "$app"
xcrun stapler validate "$app"
codesign --verify --deep --strict --verbose=2 "$app"
spctl --assess --type execute --verbose=2 "$app"
