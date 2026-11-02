# Attachment audio dependency provenance

Collected September 11–12, 2026 from the exact crates.io archives resolved in Cargo.lock.
All desktop packages include this directory at `licenses/audio`. The thirteen unmodified
Symphonia 0.6.1 source archives in `source/` provide corresponding MPL-2.0 source to
recipients; extract with `tar -xf <archive>.crate`. They retain their original licenses,
copyrights and source notices. Serein does not modify or relicense those components.

Archive URL pattern: `https://static.crates.io/crates/<crate>/<crate>-0.6.1.crate`.
SHA-256 checksums match Cargo.lock:

| Source archive | SHA-256 |
|---|---|
| symphonia-0.6.1.crate | a7edef6a96b696d4e0cab5ee9ebb7ca155ed95f30a6b45bbb8b97d2727f02424 |
| symphonia-bundle-mp3-0.6.1.crate | 98ea5ffc8716bff677dfb3b01b420c7b758de901a72b8c330bf2040ab74b4add |
| symphonia-codec-pcm-0.6.1.crate | e04ba75686acbe43542fdd374571195f0530c0b7785ca25cc6840e9c6c4b6eea |
| symphonia-codec-aac-0.6.1.crate | f5bf8e39552d34a3c4c98333370e62f48c92456d2e814f273b5c3ad7c4a5f45c |
| symphonia-core-0.6.1.crate | 01c412864d599d4750d0c3d684d7e093ec05e5309681ef5252cc1096a437f6e0 |
| symphonia-format-riff-0.6.1.crate | 1ff70929083a8c1a5f6cd7c904b6071c7914ad04739b510c2f7239dfc9b7dabe |
| symphonia-metadata-0.6.1.crate | 83713a97705d77bdef7cdbc0768fd6e5a54e4cd7e48d60a806ae85639e2c87c6 |
| symphonia-common-0.6.1.crate | 2acc3fcc18ec9b8cdd48614e259c4cf0d27b71d41e5d9b120b42c5adab12d7c4 |
| symphonia-codec-vorbis-0.6.1.crate | 73d90b4fcf796137cc683c538282804ff9629f8ad9dbfd881fcbba331ac4e986 |
| symphonia-format-ogg-0.6.1.crate | 0b5495e7f7e3c7035328d82b6d6e377eef289bb0c4105bdeb557fc93a833f994 |
| symphonia-format-isomp4-0.6.1.crate | 0e681a70e1870d34e02abf1dbc51e4267c3f1827801474e8870be8c689fc4dc3 |
| symphonia-codec-alac-0.6.1.crate | 920a78f96f3cf62932d0c497959c0cc2100ef36b03c7ca1417b260f432a482d8 |
| symphonia-bundle-flac-0.6.1.crate | 6405d5c34ff6f8f7ca08a4101efe66d21e180f7322ef9359900a0e55698a7892 |

The MP4, ALAC and FLAC archives were added October 17, 2026 for M4A/AAC/FLAC attachment playback.

Standalone texts are copied unchanged from registry source: Symphonia 0.6.1 `LICENSE`
(MPL-2.0); CPAL 0.18.2 `LICENSE` (Apache-2.0); extended 0.1.0 `LICENSE.txt` (MIT);
regex-lite 0.1.9 and lazy_static 1.5.0 `LICENSE-MIT` (MIT). CPAL was already used by
optional voice and is now also used for playback in the default build. System audio
libraries retain their own platform terms. This bundle does not replace the full
per-platform redistribution review described in THIRD_PARTY_NOTICES.md.
