# Notification sounds

The `discord/` files are byte-original assets downloaded September 21, 2026 from
Discord's public asset host. Symbolic names were verified in the public
[app bundle](https://discord.com/assets/web.9a6d63589ff469f3.js).
They are 44.1 kHz stereo MP3s, each below 128 KiB and six seconds. Playback converts
from the source sample rate; no re-encoding, metadata stripping or trimming is applied.

| Bundled file | Official source | SHA-256 |
| --- | --- | --- |
| `message.mp3` | [message1](https://discord.com/assets/3ed22d14f3c30bc4.mp3) | `31ad0482eee7770597b8aa723a80fd041ade0b076679b12293664f1f1777211b` |
| `current-channel.mp3` | [message3](https://discord.com/assets/4f53c2f31ea0cdd8.mp3) | `295dccacbdb0728ffbb8b220a278a9f3384f5966fe9b1428f33518ffa2998505` |
| `incoming-ring.mp3` | [call_ringing](https://discord.com/assets/c2a7111bb44b8da0.mp3) | `a2365a04f839099538271d06889147475ceef0845f8cc010425618f5dc412880` |
| `outgoing-ring.mp3` | [call_calling](https://discord.com/assets/5146737af413d88e.mp3) | `c3999dbbbea7fca113d6f396b81d6054dd2dd6df79f442b62dfb74afddd36934` |
| `mute.mp3` | [mute](https://discord.com/assets/2d3b4ba32c34c862.mp3) | `f6194168829b0701e8b40817d5173afed4b3b1e0b5074ab82ca31d97e4cb65c1` |
| `unmute.mp3` | [unmute](https://discord.com/assets/e74c4a06134a20e4.mp3) | `1572881f90703c1e0cd138fe7486d2e53c0ac5d8509cade32029fb31650b9304` |
| `deafen.mp3` | [deafen](https://discord.com/assets/529ff198eac567af.mp3) | `dee4468bbafb321b159dcab42f52d1fbfb1d01358437e0a3088c3345979211b8` |
| `undeafen.mp3` | [undeafen](https://discord.com/assets/b150f03c89944403.mp3) | `690b64977594baa41c7978d76259224f67799ba337df2d1045dce970ef82b243` |
| `camera-on.mp3` | [camera_on](https://discord.com/assets/855607d0932ea396.mp3) | `faeb721a072575c96d1e140aaecd469bf3f7278347596968dddf22fdb65005bf` |
| `screen-share-on.mp3` | [stream_started](https://discord.com/assets/abe52a3c92953edb.mp3) | `b5cb29d5d5cc0e8e22fa014bac4a1c2d601f6890ae6db1c18d4b6310283a3271` |
| `user-join.mp3` | [user_join](https://discord.com/assets/b135ff6c8e091b43.mp3) | `d30746caf3e4675ae0d822d51461a9ad24832afa1e20179c3c2fc7b50b911a26` |
| `user-leave.mp3` | [user_leave](https://discord.com/assets/7b9a183742515fc2.mp3) | `9fd71c2d8112c82a7fb316602bb1645bc65f5edfa260110bbaae80090fbe9df0` |

The twelve files total 595,870 bytes; each distinct cue is bundled independently. No network fetch or user-file access occurs during playback.
Incoming and outgoing rings repeat every six and three seconds respectively,
leaving enough time for each complete clip before its next playback.

These sound assets belong to Discord, Inc. and are not covered by Serein's
MIT/Apache licenses. No redistribution grant is documented in this repository;
review the [Discord terms](https://discord.com/terms) and obtain appropriate
permission before distributing builds containing them. Public download access
is not a redistribution license.
