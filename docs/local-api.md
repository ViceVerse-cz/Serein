# Local API

The local API lets another program on the same computer, such as a Stream Deck plugin, a
status light or a script, read what Serein is doing and, if the owner allows it, change
voice settings. It is Serein's own protocol, version 1. It does not talk to Discord and is
not Discord's RPC.

It is off by default. The owner turns it on in **Settings → Data & Privacy → Local API**,
which also shows the address, the access key actions and the apps that are connected.

Tested with synthetic clients only, on Windows. No third-party app has been verified
against it. The Unix socket shares its transport with game activity but is otherwise
unverified on Linux and macOS, and a Flatpak build keeps the socket in the sandbox's own
runtime directory, which a program outside the sandbox may not be able to reach.

## Security model

| Layer | What it does |
| --- | --- |
| Opt-in | Nothing is bound until the owner enables it. Turning it off closes the address and every connection. |
| No network | A named pipe on Windows and a Unix socket elsewhere. There is no TCP port, so web pages and other computers cannot connect. |
| Same user only | The pipe's access list names only the current Windows user and refuses remote clients. The socket has mode `0600` and each peer's user ID is checked. Its directory must be owned by the user or root and not writable by others; the one exception is a root-owned sticky directory such as the `/tmp` fallback, where another user could claim the name first and leave the API **Unavailable**, but still could not connect to it or pose as Serein. |
| Access key | A random 256-bit key. A client proves it holds the key with HMAC-SHA256; the key itself never crosses the connection. Serein proves it too, so a program squatting on the address cannot pose as Serein. |
| Least data | Clients get the signed-in account's ID, name and avatar hash, and the current call's members and voice flags. Never the login, messages, audio or anything about other servers. |
| Control is separate | Changing mute, deafen or a person's playback needs a second switch, **Allow apps to control voice**. |
| Bounds | Eight connections, 4 KiB requests, a five-second handshake, and no per-client queue. |

The access key is kept in the OS credential store. When no store takes it the key lasts
until Serein closes, through any number of off and on switches, and Settings says so.

**Reset key** puts a new key in force at once and disconnects every app before the store
is touched. The store then gets the new key, or failing that has the old one removed. If
it accepts neither, or never confirms a write, Serein records on this device that the
saved key is revoked (the record is made before the store is asked, so closing Serein or
switching the API off during a slow reset does not lose it): it is
never loaded again, here or after a restart, Settings says the keyring could not be
updated, and the next switch-on or reset tries again. Every credential-store call is
limited to five seconds, so a provider that stops answering cannot keep the API on, keep
old apps connected or hold up closing Serein.

This does not protect against malware already running as the same user: such a program
can read the credential store or the app's memory. Give the key only to apps you trust.

## Address

| Platform | Address |
| --- | --- |
| Windows | `\\.\pipe\serein-api` |
| Linux, macOS | `serein-api.sock` in `$XDG_RUNTIME_DIR`, else `$TMPDIR`, `$TMP`, `$TEMP`, else `/tmp` |

Settings shows the exact address with a Copy button. If another program already owns the
address, Serein reports **Unavailable** and retries every second; it never joins or
replaces that program's endpoint.

## Frames

Every message in both directions is one frame: a little-endian `u32` byte length followed
by that many bytes of UTF-8 JSON, one object per frame. Serein refuses requests over
4096 bytes. IDs are strings, because Discord IDs do not fit in a JavaScript number.

## Handshake

1. Serein sends `{"op":"hello","v":1,"nonce":"<64 hex>"}`.
2. The client answers within five seconds:
   `{"op":"auth","name":"My plugin","nonce":"<64 hex>","proof":"<64 hex>"}`.
   `nonce` is 32 fresh random bytes as hex. `name` is shown in Settings (32 characters).
3. Serein answers `{"op":"ready","v":1,"proof":"<64 hex>","topics":["session","voice"]}`.

Both proofs are HMAC-SHA256, hex encoded. The HMAC key is the access key text exactly as
copied from Settings. The message is this ASCII string, with single spaces and both
nonces exactly as they were sent:

```text
serein-local-api/1 client <hello nonce> <auth nonce>     the client's proof
serein-local-api/1 server <hello nonce> <auth nonce>     Serein's proof
```

A client should check Serein's proof before trusting anything it receives. Nothing about
the session is sent before the proof is accepted. A handshake that fails gets
`{"op":"error","code":"<code>"}` and the connection closes:

| Code | Meaning |
| --- | --- |
| `unauthorized` | The proof does not match the access key. |
| `bad_request` | The first frame is not an `auth` message, or its `nonce` is not 64 hex characters. |
| `bad_frame` | The frame is longer than 4096 bytes or is not JSON. |
| `timeout` | The handshake did not finish within five seconds. |

Test vector: key `0123456789abcdef` repeated four times, hello nonce `11` repeated 32
times, auth nonce `22` repeated 32 times.

```text
client  72021e203224508142d4cf926521ca6b9c904aae47ff176774cf3045d0fa769a
server  2e51534838c33f5e4a9ed4878544f57fc5c78e7f5c56d2e2836dc083ab1172a6
```

## Requests

A request may carry an `id` (a number or string) that the answer repeats.

| Request | Answer |
| --- | --- |
| `{"op":"subscribe","id":1,"topics":["voice"]}` | The current state of each named topic in `data`; events follow as it changes. |
| `{"op":"get","id":2,"topics":["session","voice"]}` | The same state, without subscribing. |
| `{"op":"unsubscribe","id":3,"topics":["voice"]}` | Stops that topic's events. |
| `{"op":"call","id":4,"method":"voice.set_self","params":{"mute":true}}` | Accepts or refuses a voice change. |

Answers are `{"op":"result","id":1,"ok":true,"data":{...}}` or
`{"op":"result","id":1,"ok":false,"error":"<code>"}`.

| Error | Meaning |
| --- | --- |
| `bad_request` | A field is missing or has the wrong type. |
| `unknown_op`, `unknown_topic`, `unknown_method` | The name is not part of this version. |
| `forbidden` | The owner has not allowed apps to control voice. |
| `unknown_user` | The person is not someone else in the current call. |
| `unavailable` | Nobody is signed in. |
| `busy` | Too many changes are waiting; try again. |

A frame that is too long or is not JSON gets `{"op":"error","code":"bad_frame"}` and the
connection closes.

## Topics

### `session`

```json
{ "signed_in": true, "control": false,
  "user": { "id": "10", "name": "Mina", "avatar": null } }
```

`control` is whether `call` requests are allowed. `user` is `null` when signed out.
`avatar` is Discord's avatar hash or `null`. Any change sends the whole object again as a
`state` event.

### `voice`

```json
{ "connected": true, "channel_id": "50", "guild_id": null,
  "mute": false, "deaf": false,
  "members": [
    { "user_id": "20", "name": "Ada", "avatar": null, "volume": 100,
      "muted": false, "self_mute": true, "self_deaf": false, "speaking": false }
  ] }
```

`mute` and `deaf` are the owner's own state; deafened always reads as muted. `members`
lists up to 64 people in the call, the owner included, sorted by ID. `volume` is that
person's playback on this device from 0 to 200, and `muted` is the device-only mute.
`self_mute` and `self_deaf` are what that person set. `name` is the person's nickname in
the call's own server, else their username; a nickname from another server is never
used, and it is empty when Serein has no name for them yet. Outside a call `connected`
is false, both IDs are `null` and `members` is empty.

Events arrive as `{"op":"event","topic":"voice","type":"<type>","data":{...}}`:

| Type | Data | When |
| --- | --- | --- |
| `state` | The whole object | Joining, leaving or moving to another call |
| `self` | `{ "mute", "deaf" }` | The owner's mute or deafen changed |
| `member_join` | A member | Someone joined |
| `member_update` | A member | Their name, avatar, volume or mute flags changed |
| `member_leave` | `{ "user_id" }` | Someone left |
| `speaking` | `{ "user_id", "speaking" }` | Someone started or stopped speaking |

A client that reads slowly is not sent a backlog. It receives the difference between the
last state it was told about and the newest one, so short-lived changes in between may be
skipped but the state it ends up with is always current.

## Voice control

These need **Allow apps to control voice**. `ok` means the change was accepted; the
result itself arrives as events.

| Method | Params | Effect |
| --- | --- | --- |
| `voice.set_self` | `mute` and/or `deaf` (booleans) | Mutes or deafens the owner, in or out of a call. Unmuting while deafened also undeafens, as in Discord. |
| `voice.set_user` | `user_id`, with `volume` (0–200) and/or `mute` | Sets one other call member's playback on this device. A volume outside the range is clamped. |

Neither starts or leaves a call or opens a microphone. Per-person volume and mute are
device preferences and are not sent to Discord.

A change is accepted for the call the client was looking at and is applied a moment
later. If the owner has left that call, joined another, switched account or withdrawn
control by then, or the person has left, the change is dropped silently rather than
landing somewhere else. Treat the events, not `ok`, as the result.

## Example (Node.js)

```js
const net = require("node:net");
const crypto = require("node:crypto");

const KEY = process.env.SEREIN_API_KEY; // copied from Settings → Data & Privacy → Local API
const socket = net.connect(String.raw`\\.\pipe\serein-api`);
const proof = (role, hello, mine) =>
  crypto.createHmac("sha256", KEY)
    .update(`serein-local-api/1 ${role} ${hello} ${mine}`).digest("hex");
const send = (message) => {
  const body = Buffer.from(JSON.stringify(message));
  const head = Buffer.alloc(4);
  head.writeUInt32LE(body.length);
  socket.write(Buffer.concat([head, body]));
};

let buffered = Buffer.alloc(0), hello, mine;
socket.on("data", (chunk) => {
  buffered = Buffer.concat([buffered, chunk]);
  while (buffered.length >= 4 && buffered.length >= 4 + buffered.readUInt32LE(0)) {
    const length = buffered.readUInt32LE(0);
    const message = JSON.parse(buffered.subarray(4, 4 + length));
    buffered = buffered.subarray(4 + length);
    if (message.op === "hello") {
      hello = message.nonce;
      mine = crypto.randomBytes(32).toString("hex");
      send({ op: "auth", name: "Example", nonce: mine, proof: proof("client", hello, mine) });
    } else if (message.op === "ready") {
      const expected = Buffer.from(proof("server", hello, mine), "hex");
      const given = Buffer.from(String(message.proof), "hex");
      if (given.length !== expected.length || !crypto.timingSafeEqual(given, expected)) {
        return socket.destroy(); // not Serein
      }
      send({ op: "subscribe", id: 1, topics: ["session", "voice"] });
    } else {
      console.log(message);
    }
  }
});
```

A build with the `demo` feature can serve synthetic data to develop against:
`--demo --demo-local-api` turns the API on, `--demo-local-api=control` also allows voice
control, and `--demo-voice` adds a call. In the demo the access key is not saved and
changes every launch.
