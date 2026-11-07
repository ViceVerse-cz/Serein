# Serein Voice (phone app)

Read-only companion for Serein's **Phone bridge**. The computer forwards incoming chat
messages over the local network; this app shows one row per server channel or friend and
the messages that arrive for it. It never sends anything to Discord.

## Run

1. On the computer, open **Settings → Voice & Video**, enable **Phone bridge**, and note
   the port and token. Serein listens on the LAN while it is running.
2. In this folder run `npm install` once, then `npx expo start --lan`.
3. On the iPhone, install **Expo Go** from the App Store and scan the QR code.
4. Enter the computer's Wi-Fi address, the port and the token, then **Connect**.

The app is TypeScript only; a native build (`npx expo run:ios`) needs macOS and Xcode,
which Expo Go avoids.

## Protocol

`ws://<computer>:<port>/?token=<token>`, JSON text frames:

```json
{"type":"channel","channel":{"id":"1","kind":0,"name":"general","guild":"My Server","voice":false}}
{"type":"message","channel":{...},"message":{"id":"9","author":"alice","content":"hi","edited":false}}
```

The token must match exactly; a wrong token is rejected during the WebSocket handshake.
Serein bounds the payloads: 512 characters of content, 96 characters of names, 200
channels, four phones, and a 128-frame backlog per phone.

## Building a real app icon (no Mac here)

`.github/workflows/phone-ios.yml` builds an **unsigned** `.ipa` on a GitHub macOS
runner (`workflow_dispatch`, or a push touching `apps/phone`). Download the
`SereinVoice-unsigned-ipa` artifact and sign it for the device with Sideloadly and your
own Apple ID; the repository holds no signing secrets.

XcodeGen is not used: `expo prebuild` generates the Xcode project together with the
CocoaPods integration React Native needs, and a hand-written XcodeGen spec would have to
reproduce those build phases. The workflow discovers the generated workspace and scheme
instead of hardcoding them.

## Installing the unsigned build on the phone (Windows, no Mac)

1. `winget install Apple.AppleMobileDeviceSupport` (the Apple Mobile Device Service is what
   talks to the phone; iTunes alone no longer installs it).
2. Plug the iPhone in, unlock it, and tap **Trust**. If `ListDevices` on usbmuxd stays empty,
   disable and re-enable the `USB\VID_05AC&PID_12A8\…` device in Device Manager.
3. Install [Sideloadly](https://sideloadly.io/), open `SereinVoice-unsigned.ipa` with it, pick
   the device, enter your own Apple ID and press **Start**. Sideloadly signs the bundle with a
   free-account profile (valid 7 days) and installs it over USB.
