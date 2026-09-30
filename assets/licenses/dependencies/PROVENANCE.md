# Additional dependency notice and source provenance

Collected September 11, 2026 against the locked dependency graph at `11d0416`.
Files below are unmodified copies from cached registry archives, the pinned egui checkout,
or exact upstream commits identified by each released crate's `.cargo_vcs_info.json`.
Registry archive SHA-256 values were matched to `Cargo.lock` before copying source archives.
These supplements are bundled without dependency selection or coverage checks; this directory
is not an assertion that every listed package belongs in every binary.

The `.crate` files are unmodified corresponding MPL component sources. They can be extracted
with `tar`; their original license terms remain separate from Serein. Existing Symphonia
archives under `assets/licenses/audio/source` and modified `vendor/hpke-rs` are reused.
The canonical MPL text already retained for hpke-rs is also used for its two registry providers,
whose exact upstream release tree omits a standalone license file.

Nested egui font notices supplement its workspace MIT/Apache texts. The egui icon subset
provenance describes its upstream modification; bundled OS fonts are not claimed. AWS-LC's
fiat notice supplements its composite root LICENSE, which the collector obtains from the
registry package. Existing direct-component notices are reused where their provenance applies.

`objc2-*-LICENSE.md` files are upstream licensing/SDK notices containing external links,
not complete license grants by themselves. Their presence must not close a missing-text
check. Older `objc2-*-LICENSE.txt` files contain the upstream MIT text.

## Payloads

| File | Component(s) | Exact source | SHA-256 |
| --- | --- | --- | --- |
| `egui-LICENSE-MIT` | egui workspace 0.36.2 at 65e7db3c06d779c60ac56647bdd3011ed8ba1cbd | [upstream](https://github.com/emilk/egui/blob/65e7db3c06d779c60ac56647bdd3011ed8ba1cbd/LICENSE-MIT) | `95ca92f5f8ea5231f1580b3a2a799e8260af3114b900e1def5355a7f44bcf60c` |
| `egui-LICENSE-APACHE` | egui workspace 0.36.2 at 65e7db3c06d779c60ac56647bdd3011ed8ba1cbd | [upstream](https://github.com/emilk/egui/blob/65e7db3c06d779c60ac56647bdd3011ed8ba1cbd/LICENSE-APACHE) | `8173d5c29b4f956d532781d2b86e4e30f83e6b7878dce18c919451d6ba707c90` |
| `egui-fonts-Hack-Regular.txt` | epaint_default_fonts 0.36.2 | [upstream](https://github.com/emilk/egui/blob/65e7db3c06d779c60ac56647bdd3011ed8ba1cbd/crates/epaint_default_fonts/fonts/Hack-Regular.txt) | `47c0cccbeec7e8614548cc485588b28149e7874188df5f41b36efebcee285c87` |
| `egui-fonts-OFL.txt` | epaint_default_fonts 0.36.2 | [upstream](https://github.com/emilk/egui/blob/65e7db3c06d779c60ac56647bdd3011ed8ba1cbd/crates/epaint_default_fonts/fonts/OFL.txt) | `6a73f9541c2de74158c0e7cf6b0a58ef774f5a780bf191f2d7ec9cc53efe2bf2` |
| `egui-fonts-UFL.txt` | epaint_default_fonts 0.36.2 | [upstream](https://github.com/emilk/egui/blob/65e7db3c06d779c60ac56647bdd3011ed8ba1cbd/crates/epaint_default_fonts/fonts/UFL.txt) | `2f0015108d68627bd788d313f529c21ff4da2c2c42a5e1f3883acc83480f9002` |
| `egui-fonts-emoji-icon-font-mit-license.txt` | epaint_default_fonts 0.36.2 | [upstream](https://github.com/emilk/egui/blob/65e7db3c06d779c60ac56647bdd3011ed8ba1cbd/crates/epaint_default_fonts/fonts/emoji-icon-font-mit-license.txt) | `b9d2c1d909aa149996fd4c91dcb92b2362a04431640c1d200959da94caf8cde1` |
| `egui-fonts-egui-icons.txt` | epaint_default_fonts 0.36.2 | [upstream](https://github.com/emilk/egui/blob/65e7db3c06d779c60ac56647bdd3011ed8ba1cbd/crates/epaint_default_fonts/fonts/egui-icons.txt) | `36dbbfd79d73974f864116879c17dd11559e1340277385f1ac52d8d29a5f5c79` |
| `aws-lc-fiat-LICENSE` | aws-lc-sys 0.45.0 | [upstream](https://docs.rs/crate/aws-lc-sys/0.45.0/source/aws-lc/third_party/fiat/LICENSE) | `43e358d7b6eb109d0f51f7b3a090fd82607965767c25fadee39e922475de2061` |
| `option-ext-0.2.0.crate` | option-ext 0.2.0 | [upstream](https://static.crates.io/crates/option-ext/option-ext-0.2.0.crate) | `04744f49eae99ab78e0d5c0b603ab218f515ea8cfe5a456d7629ad883a3b6e7d` |
| `hpke-rs-crypto-0.6.1.crate` | hpke-rs-crypto 0.6.1 | [upstream](https://static.crates.io/crates/hpke-rs-crypto/hpke-rs-crypto-0.6.1.crate) | `0a73a99d9008010d73289f41335a3f6e14fb8c04eaf60e9111b450463b1bbc7f` |
| `hpke-rs-rust-crypto-0.6.1.crate` | hpke-rs-rust-crypto 0.6.1 | [upstream](https://static.crates.io/crates/hpke-rs-rust-crypto/hpke-rs-rust-crypto-0.6.1.crate) | `14b28be6cba9081c7feda2651d51c2a900029798e78b4c1e093e792f4571a870` |
| `accesskit-a55d3e1a-LICENSE-MIT` | accesskit 0.24.1 | [upstream](https://raw.githubusercontent.com/AccessKit/accesskit/a55d3e1a18bb9ef0e4bccc9083fb13c3e0ad8969/LICENSE-MIT) | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| `accesskit-a55d3e1a-LICENSE-APACHE` | accesskit 0.24.1 | [upstream](https://raw.githubusercontent.com/AccessKit/accesskit/a55d3e1a18bb9ef0e4bccc9083fb13c3e0ad8969/LICENSE-APACHE) | `62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a` |
| `accesskit-f40dfc01-LICENSE-MIT` | accesskit_atspi_common 0.18.1; accesskit_consumer 0.36.0; accesskit_unix 0.21.1 | [upstream](https://raw.githubusercontent.com/AccessKit/accesskit/f40dfc01a0c0e76de535969f82fb35e19513737d/LICENSE-MIT) | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| `accesskit-f40dfc01-LICENSE-APACHE` | accesskit_atspi_common 0.18.1; accesskit_consumer 0.36.0; accesskit_unix 0.21.1 | [upstream](https://raw.githubusercontent.com/AccessKit/accesskit/f40dfc01a0c0e76de535969f82fb35e19513737d/LICENSE-APACHE) | `62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a` |
| `accesskit-1bbcf100-LICENSE-MIT` | accesskit_consumer 0.35.0; accesskit_windows 0.32.1; accesskit_winit 0.32.2 | [upstream](https://raw.githubusercontent.com/AccessKit/accesskit/1bbcf100942bac96c2c3a4a91cb67b0b20201a24/LICENSE-MIT) | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| `accesskit-1bbcf100-LICENSE-APACHE` | accesskit_consumer 0.35.0; accesskit_windows 0.32.1; accesskit_winit 0.32.2 | [upstream](https://raw.githubusercontent.com/AccessKit/accesskit/1bbcf100942bac96c2c3a4a91cb67b0b20201a24/LICENSE-APACHE) | `62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a` |
| `accesskit-c88605b9-LICENSE-MIT` | accesskit_consumer 0.38.0; accesskit_macos 0.26.3 | [upstream](https://raw.githubusercontent.com/AccessKit/accesskit/c88605b96d04431f9c3c792464a0f2f253480e94/LICENSE-MIT) | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| `accesskit-c88605b9-LICENSE-APACHE` | accesskit_consumer 0.38.0; accesskit_macos 0.26.3 | [upstream](https://raw.githubusercontent.com/AccessKit/accesskit/c88605b96d04431f9c3c792464a0f2f253480e94/LICENSE-APACHE) | `62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a` |
| `objc2-4fc083f1-LICENSE.txt` | block2 0.5.1; objc-sys 0.3.5; objc2 0.5.2 | [upstream](https://raw.githubusercontent.com/madsmtm/objc2/4fc083f1c6d6784577e38b0ee8dbd344481e2fd2/LICENSE.txt) | `e353f37b12aefbb9f9b29490e837cfee05d9bda70804b3562839a3285c1df1e5` |
| `objc2-b4167b58-LICENSE.md` | block2 0.6.2 | [upstream](https://raw.githubusercontent.com/madsmtm/objc2/b4167b582b2f75f9a1be75495c41b765344fd03c/LICENSE.md) | `7f976f7e9cb2d87df7230606feb932c3f21ac0e664045a775b600046ff850c54` |
| `clipboard-win-3b27cf2b-LICENSE` | clipboard-win 5.4.1 | [upstream](https://raw.githubusercontent.com/DoumanAsh/clipboard-win/3b27cf2bfd1adcfa6e0264eb51c1025ddaf0f342/LICENSE) | `c9bff75738922193e67fa726fa225535870d2aa1059f91452c411736284ad566` |
| `sample-97c3bb9b-LICENSE-MIT` | dasp_sample 0.11.0 | [upstream](https://raw.githubusercontent.com/rustaudio/sample/97c3bb9b2363c0b46ac1633858bf1054fd02a980/LICENSE-MIT) | `b1d6df41ed3aa96806e74c729444d7c121d90e6660a6aed01d298e03fde475a0` |
| `sample-97c3bb9b-LICENSE-APACHE` | dasp_sample 0.11.0 | [upstream](https://raw.githubusercontent.com/rustaudio/sample/97c3bb9b2363c0b46ac1633858bf1054fd02a980/LICENSE-APACHE) | `756c8d2ab2dc24f256e1455b5f03937f4933d83d7bc9f2d55009fc962377d512` |
| `gl-rs-ea503e8d-LICENSE` | gl_generator 0.14.0 | [upstream](https://raw.githubusercontent.com/brendanzab/gl-rs/ea503e8d5fb6d73c6030e6191ce738cd3bf3433e/LICENSE) | `cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30` |
| `gl-rs-f150967b-LICENSE` | khronos_api 3.1.0 | [upstream](https://raw.githubusercontent.com/brendanzab/gl-rs/f150967b1c44ae888e6676f93f639ebc82771bdc/LICENSE) | `cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30` |
| `objc2-8852b424-LICENSE.md` | objc2 0.6.4 | [upstream](https://raw.githubusercontent.com/madsmtm/objc2/8852b424193ca41602281b3d7540d7c8ed51e49a/LICENSE.md) | `7f976f7e9cb2d87df7230606feb932c3f21ac0e664045a775b600046ff850c54` |
| `objc2-e282618b-LICENSE.txt` | objc2-app-kit 0.2.2; objc2-cloud-kit 0.2.2; objc2-contacts 0.2.2; objc2-core-data 0.2.2; objc2-core-image 0.2.2; objc2-core-location 0.2.2; objc2-foundation 0.2.2; objc2-link-presentation 0.2.2; objc2-metal 0.2.2; objc2-quartz-core 0.2.2; objc2-symbols 0.2.2; objc2-ui-kit 0.2.2; objc2-uniform-type-identifiers 0.2.2; objc2-user-notifications 0.2.2 | [upstream](https://raw.githubusercontent.com/madsmtm/objc2/e282618be4c3a3b9542957e0c8540e9588472ce8/LICENSE.txt) | `e353f37b12aefbb9f9b29490e837cfee05d9bda70804b3562839a3285c1df1e5` |
| `objc2-7b1abfd7-LICENSE.md` | objc2-app-kit 0.3.2; objc2-audio-toolbox 0.3.2; objc2-av-foundation 0.3.2; objc2-avf-audio 0.3.2; objc2-core-audio 0.3.2; objc2-core-audio-types 0.3.2; objc2-core-foundation 0.3.2; objc2-core-graphics 0.3.2; objc2-core-location 0.3.2; objc2-core-text 0.3.2; objc2-foundation 0.3.2; objc2-io-surface 0.3.2; objc2-metal 0.3.2; objc2-quartz-core 0.3.2; objc2-ui-kit 0.3.2; objc2-user-notifications 0.3.2; objc2-web-kit 0.3.2 | [upstream](https://raw.githubusercontent.com/madsmtm/objc2/7b1abfd750a2cacaea71d6a56ecfb83cb7de560b/LICENSE.md) | `7f976f7e9cb2d87df7230606feb932c3f21ac0e664045a775b600046ff850c54` |
| `objc2-8d214f54-LICENSE.md` | objc2-encode 4.1.0; objc2-exception-helper 0.1.1 | [upstream](https://raw.githubusercontent.com/madsmtm/objc2/8d214f5477365ffcbcbb7de058c86ed9a518efb7/LICENSE.md) | `7f976f7e9cb2d87df7230606feb932c3f21ac0e664045a775b600046ff850c54` |
| `openmls-47dbedec-LICENSE` | openmls 0.8.1; openmls_rust_crypto 0.5.1 | [upstream](https://raw.githubusercontent.com/openmls/openmls/47dbedecad0c1fd8eb5368d582250ebfcc1e1ce6/LICENSE) | `43e5e3c4b5cca67f9ea912f7e1929702a848aa765d3b3e14f25d3838a5a5565d` |
| `openmls-6b85f0ed-LICENSE` | openmls_basic_credential 0.5.0; openmls_memory_storage 0.5.0; openmls_traits 0.5.0 | [upstream](https://raw.githubusercontent.com/openmls/openmls/6b85f0edc560b4fe0f5b9266092947a774614f3f/LICENSE) | `43e5e3c4b5cca67f9ea912f7e1929702a848aa765d3b3e14f25d3838a5a5565d` |
| `profiling-82715511-LICENSE-MIT` | profiling 1.0.18 | [upstream](https://raw.githubusercontent.com/aclysma/profiling/8271551172eb6fa4cba47369aedd93790c623df9/LICENSE-MIT) | `c8167fdeeed46d3f244d3f85c5bf998ce889343691c32be2c61a8bc4b5c08333` |
| `profiling-82715511-LICENSE-APACHE` | profiling 1.0.18 | [upstream](https://raw.githubusercontent.com/aclysma/profiling/8271551172eb6fa4cba47369aedd93790c623df9/LICENSE-APACHE) | `10d30a673cd5e9349bdc02aeb48f14b3386d27d0da32df8f0a555d4aa16aa551` |
| `rspirv-8afc3d0a-LICENSE` | spirv 0.4.0+sdk-1.4.341.0 | [upstream](https://raw.githubusercontent.com/gfx-rs/rspirv/8afc3d0ac8e158128cd1410bb2e4b4c26ab11bb4/LICENSE) | `cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30` |
| `webview2-rs-edc2caf8-LICENSE` | webview2-com 0.39.1; webview2-com-sys 0.39.1 | [upstream](https://raw.githubusercontent.com/wravery/webview2-rs/edc2caf886175ccaebe86078c9cfe1ae2a187328/LICENSE) | `0dcf41516e608bbcb6cdc5229feb7b86fe4a643b85e7df251133c93408fdac73` |
| `webview2-rs-dffa41a8-LICENSE` | webview2-com-macros 0.8.1 | [upstream](https://raw.githubusercontent.com/wravery/webview2-rs/dffa41a8a46d3f5565eefbff2de57d38d399f158/LICENSE) | `0dcf41516e608bbcb6cdc5229feb7b86fe4a643b85e7df251133c93408fdac73` |

## Released archive identities

These are the Cargo.lock archive checksums associated with downloaded or cached release
notices. Git-sourced egui uses the full revision recorded in its source links instead.

- 04744f49eae99ab78e0d5c0b603ab218f515ea8cfe5a456d7629ad883a3b6e7d
- 0a73a99d9008010d73289f41335a3f6e14fb8c04eaf60e9111b450463b1bbc7f
- 14b28be6cba9081c7feda2651d51c2a900029798e78b4c1e093e792f4571a870
- 9bff6c3b54fad79a2e60b8102caf565819711497c1f5f092f49508e2f5c31b27
- accesskit 0.24.1: d3b7f7f85a7e5f68090000ed7622545829afd484d210358702ae4cb97dd0c320
- accesskit_atspi_common 0.18.1: 1e8c61bee90b42a772d39d06a740207dc71a4e780004ace1db8d99fb1baaa954
- accesskit_consumer 0.35.0: 53cf47daed85312e763fbf85ceca136e0d7abc68e0a7e12abe11f48172bc3b10
- accesskit_consumer 0.36.0: 25e0d7e25d06f4dc21d1774d67146e9e80d6789216cbd4d1e88185b0095dba60
- accesskit_consumer 0.38.0: 5d10a236f96f87d70732e44520046785431ef01d5bcd6b041317bfadd2f88245
- accesskit_macos 0.26.3: ce02dc63b43f0c9296af9ac946312a2dc8814427d7a64d2d600971dac55b6076
- accesskit_unix 0.21.1: b016ca8db0ea0ea2ceff29a9d6240391492d960716aa471967c00e8cc8cb197c
- accesskit_windows 0.32.1: eff7009f1a532e917d66970a1e80c965140c6cfbbabbdde3d64e5431e6c78e21
- accesskit_winit 0.32.2: 1fe9a94394896352cc4660ca2288bd4ef883d83238853c038b44070c8f134313
- block2 0.5.1: 2c132eebf10f5cad5289222520a4a058514204aed6d791f1cf4fe8088b82d15f
- block2 0.6.2: cdeb9d870516001442e364c5220d3574d2da8dc765554b4a617230d33fa58ef5
- clipboard-win 5.4.1: bde03770d3df201d4fb868f2c9c59e66a3e4e2bd06692a0fe701e7103c7e84d4
- dasp_sample 0.11.0: 0c87e182de0887fd5361989c677c4e8f5000cd9491d6d563161a8f3a5519fc7f
- gl_generator 0.14.0: 1a95dfc23a2b4a9a2f5ab41d194f8bfda3cabec42af4e39f08c339eb2a0c124d
- khronos_api 3.1.0: e2db585e1d738fc771bf08a151420d3ed193d9d895a36df7f6f8a9456b911ddc
- objc-sys 0.3.5: cdb91bdd390c7ce1a8607f35f3ca7151b65afc0ff5ff3b34fa350f7d7c7e4310
- objc2 0.5.2: 46a785d4eeff09c14c487497c162e92766fbb3e4059a71840cecc03d9a50b804
- objc2 0.6.4: 3a12a8ed07aefc768292f076dc3ac8c48f3781c8f2d5851dd3d98950e8c5a89f
- objc2-app-kit 0.2.2: e4e89ad9e3d7d297152b17d39ed92cd50ca8063a89a9fa569046d41568891eff
- objc2-app-kit 0.3.2: d49e936b501e5c5bf01fda3a9452ff86dc3ea98ad5f283e1455153142d97518c
- objc2-audio-toolbox 0.3.2: 6948501a91121d6399b79abaa33a8aa4ea7857fe019f341b8c23ad6e81b79b08
- objc2-av-foundation 0.3.2: 478ae33fcac9df0a18db8302387c666b8ef08a3e2d62b510ca4fc278a384b6c0
- objc2-avf-audio 0.3.2: 13a380031deed8e99db00065c45937da434ca987c034e13b87e4441f9e4090be
- objc2-cloud-kit 0.2.2: 74dd3b56391c7a0596a295029734d3c1c5e7e510a4cb30245f8221ccea96b009
- objc2-contacts 0.2.2: a5ff520e9c33812fd374d8deecef01d4a840e7b41862d849513de77e44aa4889
- objc2-core-audio 0.3.2: e1eebcea8b0dbff5f7c8504f3107c68fc061a3eb44932051c8cf8a68d969c3b2
- objc2-core-audio-types 0.3.2: 5a89f2ec274a0cf4a32642b2991e8b351a404d290da87bb6a9a9d8632490bd1c
- objc2-core-data 0.2.2: 617fbf49e071c178c0b24c080767db52958f716d9eabdf0890523aeae54773ef
- objc2-core-foundation 0.3.2: 2a180dd8642fa45cdb7dd721cd4c11b1cadd4929ce112ebd8b9f5803cc79d536
- objc2-core-graphics 0.3.2: e022c9d066895efa1345f8e33e584b9f958da2fd4cd116792e15e07e4720a807
- objc2-core-image 0.2.2: 55260963a527c99f1819c4f8e3b47fe04f9650694ef348ffd2227e8196d34c80
- objc2-core-location 0.2.2: 000cfee34e683244f284252ee206a27953279d370e309649dc3ee317b37e5781
- objc2-core-location 0.3.2: ca347214e24bc973fc025fd0d36ebb179ff30536ed1f80252706db19ee452009
- objc2-core-text 0.3.2: 0cde0dfb48d25d2b4862161a4d5fcc0e3c24367869ad306b0c9ec0073bfed92d
- objc2-encode 4.1.0: ef25abbcd74fb2609453eb695bd2f860d389e457f67dc17cafc8b8cbc89d0c33
- objc2-exception-helper 0.1.1: c7a1c5fbb72d7735b076bb47b578523aedc40f3c439bea6dfd595c089d79d98a
- objc2-foundation 0.2.2: 0ee638a5da3799329310ad4cfa62fbf045d5f56e3ef5ba4149e7452dcf89d5a8
- objc2-foundation 0.3.2: e3e0adef53c21f888deb4fa59fc59f7eb17404926ee8a6f59f5df0fd7f9f3272
- objc2-io-surface 0.3.2: 180788110936d59bab6bd83b6060ffdfffb3b922ba1396b312ae795e1de9d81d
- objc2-link-presentation 0.2.2: a1a1ae721c5e35be65f01a03b6d2ac13a54cb4fa70d8a5da293d7b0020261398
- objc2-metal 0.2.2: dd0cba1276f6023976a406a14ffa85e1fdd19df6b0f737b063b95f6c8c7aadd6
- objc2-metal 0.3.2: a0125f776a10d00af4152d74616409f0d4a2053a6f57fa5b7d6aa2854ac04794
- objc2-quartz-core 0.2.2: e42bee7bff906b14b167da2bac5efe6b6a07e6f7c0a21a7308d40c960242dc7a
- objc2-quartz-core 0.3.2: 96c1358452b371bf9f104e21ec536d37a650eb10f7ee379fff67d2e08d537f1f
- objc2-symbols 0.2.2: 0a684efe3dec1b305badae1a28f6555f6ddd3bb2c2267896782858d5a78404dc
- objc2-ui-kit 0.2.2: b8bb46798b20cd6b91cbd113524c490f1686f4c4e8f49502431415f3512e2b6f
- objc2-ui-kit 0.3.2: d87d638e33c06f577498cbcc50491496a3ed4246998a7fbba7ccb98b1e7eab22
- objc2-uniform-type-identifiers 0.2.2: 44fa5f9748dbfe1ca6c0b79ad20725a11eca7c2218bceb4b005cb1be26273bfe
- objc2-user-notifications 0.2.2: 76cfcbf642358e8689af64cee815d139339f3ed8ad05103ed5eaf73db8d84cb3
- objc2-user-notifications 0.3.2: 9df9128cbbfef73cda168416ccf7f837b62737d748333bfe9ab71c245d76613e
- objc2-web-kit 0.3.2: b2e5aaab980c433cf470df9d7af96a7b46a9d892d521a2cbbb2f8a4c16751e7f
- openmls 0.8.1: dcb512bfe6a55777518853ea535c6241f069cb0e8984678c117151d2a1e7e903
- openmls_basic_credential 0.5.0: 983e8be1457dd6f316f409292cec334af3b57b49a19deadc925c83c3c35e15b6
- openmls_memory_storage 0.5.0: 1a52c927ddb9940acb96d51aebd54b8b9c601c7119e6609622fb3f2cbe16abe3
- openmls_rust_crypto 0.5.1: fafcc8a3552b10fbb3ab757cccaf1a34081e826ca819f49aa7e6645b1d95c00f
- openmls_traits 0.5.0: 4f88ccdd53448dfdbfa5b8da8ba4e527c418fdb966418172bace2e3b41eedd56
- profiling 1.0.18: 3d595e54a326bc53c1c197b32d295e14b169e3cfeaa8dc82b529f947fba6bcf5
- spirv 0.4.0+sdk-1.4.341.0: d9571ea910ebd84c86af4b3ed27f9dbdc6ad06f17c5f96146b2b671e2976744f
- webview2-com 0.39.1: 3f89fca7a704cee10dcb3654c1dbb8941d1783132f1917358af75bec37a7d7e6
- webview2-com-macros 0.8.1: 67a921c1b6914c367b2b823cd4cde6f96beec77d30a939c8199bb377cf9b9b54
- webview2-com-sys 0.39.1: b3a07132775117d6065853d9d1178157b8c90e228de47129d6bce2c7edebedfb

## Unresolved upstream omissions

- `realfft 3.5.0`: the registry source and exact upstream tree at
  `d0d4eee0525fd27c96c8a046d6d107acd5ed84a6` declare MIT but contain no standalone
  license grant/copyright notice. [Exact tree](https://github.com/HEnquist/realfft/tree/d0d4eee0525fd27c96c8a046d6d107acd5ed84a6).
- `dispatch 0.2.0`: the registry source and exact upstream tree at
  `82d6c7a5b75dc0c71c3f46f87bb6c16a476f7748` declare MIT but contain no standalone
  license grant/copyright notice. [Exact tree](https://github.com/SSheldon/rust-dispatch/tree/82d6c7a5b75dc0c71c3f46f87bb6c16a476f7748).

No copyright holder or replacement grant has been invented for either omission. Neither
an SPDX declaration nor passing automated checks establishes complete redistribution clearance.
System SDK/runtime terms and independent legal/patent review remain outside this text-assembly check.

## Explicit unresolved notice records

The MIT reference is the unmodified SPDX reference text, including its placeholders. It is
not a project copyright notice. The exact release source archives and original declarations
are supplied for realfft/dispatch and the modern objc2 family, alongside upstream notices.
The overrides mark these exact package/source versions unresolved; their inventories remain
`complete: false` and packaging warns. No copyright holder or missing grant is invented.
This makes development-package evidence reviewable; full notice/redistribution review remains
an incomplete spec gate. New unlisted missing notices still stop packaging.

| File | Component | Exact source | SHA-256 |
| --- | --- | --- | --- |
| `MIT-reference.txt` | Reference terms only; not an invented project copyright notice | [source](https://raw.githubusercontent.com/spdx/license-list-data/16f3aa6c3bdd62e50f8b1cf618f32d2a510250ee/text/MIT.txt) | `b05785f9f18e6716bab63424b11454513b9943a222595b70411009202fc592b5` |
| `block2-0.6.2.crate` | block2 0.6.2 | [source](https://static.crates.io/crates/block2/block2-0.6.2.crate) | `cdeb9d870516001442e364c5220d3574d2da8dc765554b4a617230d33fa58ef5` |
| `dispatch-0.2.0.crate` | dispatch 0.2.0 | [source](https://static.crates.io/crates/dispatch/dispatch-0.2.0.crate) | `bd0c93bb4b0c6d9b77f4435b0ae98c24d17f1c45b2ff844c6151a07256ca923b` |
| `dispatch-0.2.0-license-declaration.toml` | dispatch 0.2.0 | [source](https://docs.rs/crate/dispatch/0.2.0/source/Cargo.toml.orig) | `05c60b829931ef7f1f130735843c11f5cf9cc69546eca5fc941d014781c68e31` |
| `dispatch2-0.3.1.crate` | dispatch2 0.3.1 | [source](https://static.crates.io/crates/dispatch2/dispatch2-0.3.1.crate) | `1e0e367e4e7da84520dedcac1901e4da967309406d1e51017ae1abfb97adbd38` |
| `objc2-0.6.4.crate` | objc2 0.6.4 | [source](https://static.crates.io/crates/objc2/objc2-0.6.4.crate) | `3a12a8ed07aefc768292f076dc3ac8c48f3781c8f2d5851dd3d98950e8c5a89f` |
| `objc2-app-kit-0.3.2.crate` | objc2-app-kit 0.3.2 | [source](https://static.crates.io/crates/objc2-app-kit/objc2-app-kit-0.3.2.crate) | `d49e936b501e5c5bf01fda3a9452ff86dc3ea98ad5f283e1455153142d97518c` |
| `objc2-audio-toolbox-0.3.2.crate` | objc2-audio-toolbox 0.3.2 | [source](https://static.crates.io/crates/objc2-audio-toolbox/objc2-audio-toolbox-0.3.2.crate) | `6948501a91121d6399b79abaa33a8aa4ea7857fe019f341b8c23ad6e81b79b08` |
| `objc2-av-foundation-0.3.2.crate` | objc2-av-foundation 0.3.2 | [source](https://static.crates.io/crates/objc2-av-foundation/objc2-av-foundation-0.3.2.crate) | `478ae33fcac9df0a18db8302387c666b8ef08a3e2d62b510ca4fc278a384b6c0` |
| `objc2-core-audio-0.3.2.crate` | objc2-core-audio 0.3.2 | [source](https://static.crates.io/crates/objc2-core-audio/objc2-core-audio-0.3.2.crate) | `e1eebcea8b0dbff5f7c8504f3107c68fc061a3eb44932051c8cf8a68d969c3b2` |
| `objc2-core-audio-types-0.3.2.crate` | objc2-core-audio-types 0.3.2 | [source](https://static.crates.io/crates/objc2-core-audio-types/objc2-core-audio-types-0.3.2.crate) | `5a89f2ec274a0cf4a32642b2991e8b351a404d290da87bb6a9a9d8632490bd1c` |
| `objc2-core-foundation-0.3.2.crate` | objc2-core-foundation 0.3.2 | [source](https://static.crates.io/crates/objc2-core-foundation/objc2-core-foundation-0.3.2.crate) | `2a180dd8642fa45cdb7dd721cd4c11b1cadd4929ce112ebd8b9f5803cc79d536` |
| `objc2-core-graphics-0.3.2.crate` | objc2-core-graphics 0.3.2 | [source](https://static.crates.io/crates/objc2-core-graphics/objc2-core-graphics-0.3.2.crate) | `e022c9d066895efa1345f8e33e584b9f958da2fd4cd116792e15e07e4720a807` |
| `objc2-core-location-0.3.2.crate` | objc2-core-location 0.3.2 | [source](https://static.crates.io/crates/objc2-core-location/objc2-core-location-0.3.2.crate) | `ca347214e24bc973fc025fd0d36ebb179ff30536ed1f80252706db19ee452009` |
| `objc2-core-text-0.3.2.crate` | objc2-core-text 0.3.2 | [source](https://static.crates.io/crates/objc2-core-text/objc2-core-text-0.3.2.crate) | `0cde0dfb48d25d2b4862161a4d5fcc0e3c24367869ad306b0c9ec0073bfed92d` |
| `objc2-encode-4.1.0.crate` | objc2-encode 4.1.0 | [source](https://static.crates.io/crates/objc2-encode/objc2-encode-4.1.0.crate) | `ef25abbcd74fb2609453eb695bd2f860d389e457f67dc17cafc8b8cbc89d0c33` |
| `objc2-exception-helper-0.1.1.crate` | objc2-exception-helper 0.1.1 | [source](https://static.crates.io/crates/objc2-exception-helper/objc2-exception-helper-0.1.1.crate) | `c7a1c5fbb72d7735b076bb47b578523aedc40f3c439bea6dfd595c089d79d98a` |
| `objc2-foundation-0.3.2.crate` | objc2-foundation 0.3.2 | [source](https://static.crates.io/crates/objc2-foundation/objc2-foundation-0.3.2.crate) | `e3e0adef53c21f888deb4fa59fc59f7eb17404926ee8a6f59f5df0fd7f9f3272` |
| `objc2-metal-0.3.2.crate` | objc2-metal 0.3.2 | [source](https://static.crates.io/crates/objc2-metal/objc2-metal-0.3.2.crate) | `a0125f776a10d00af4152d74616409f0d4a2053a6f57fa5b7d6aa2854ac04794` |
| `objc2-quartz-core-0.3.2.crate` | objc2-quartz-core 0.3.2 | [source](https://static.crates.io/crates/objc2-quartz-core/objc2-quartz-core-0.3.2.crate) | `96c1358452b371bf9f104e21ec536d37a650eb10f7ee379fff67d2e08d537f1f` |
| `objc2-user-notifications-0.3.2.crate` | objc2-user-notifications 0.3.2 | [source](https://static.crates.io/crates/objc2-user-notifications/objc2-user-notifications-0.3.2.crate) | `9df9128cbbfef73cda168416ccf7f837b62737d748333bfe9ab71c245d76613e` |
| `objc2-web-kit-0.3.2.crate` | objc2-web-kit 0.3.2 | [source](https://static.crates.io/crates/objc2-web-kit/objc2-web-kit-0.3.2.crate) | `b2e5aaab980c433cf470df9d7af96a7b46a9d892d521a2cbbb2f8a4c16751e7f` |
| `realfft-3.5.0.crate` | realfft 3.5.0 | [source](https://static.crates.io/crates/realfft/realfft-3.5.0.crate) | `f821338fddb99d089116342c46e9f1fbf3828dba077674613e734e01d6ea8677` |
| `realfft-3.5.0-license-declaration.toml` | realfft 3.5.0 | [source](https://docs.rs/crate/realfft/3.5.0/source/Cargo.toml.orig) | `d72ddbadf9bb55ed21ae973ac97f0bb4e8df2064af628c54b802b2c7d764c8de` |

## DeepFilterNet inference dependencies — September 29, 2026

The following 52 registry releases were added relative to baseline
`306bccdbb4d28fa83dac09260772917d3d8b0018`; the list includes the Tract
0.22.4 inference family and its build-time code-generation helpers. It is the
lockfile delta, not a claim that every package is linked into every artifact.
No existing dependency notice is replaced.

Each cached `.crate` archive was SHA-256 checked against the current `Cargo.lock`.
License, copyright and notice files below were copied byte-for-byte from those
verified archives and compared with the corresponding extracted cached source.
`nom-language 0.1.0` omits its license text from the archive; its MIT text is
copied from the root of rust-bakery/nom at the exact release VCS revision
`2cec1b3e4c9ccac62c902d60c00de6d1549ccbe1`. Liquid-lib's archive includes its
MIT text; no separate Apache text is invented for that component.

DeepFilterNet's vendored runtime, original repository licenses and embedded model
provenance are recorded separately in `assets/licenses/voice/PROVENANCE.md`.
The outstanding pretrained-model license-scope clarification is not resolved by
these dependency notices. Existing full-artifact evidence gaps remain unchanged.

### Added registry archive identities

| Component | Version | Declared license | Archive SHA-256 |
| --- | --- | --- | --- |
| `anyhow` | `1.0.104` | `MIT OR Apache-2.0` | `330a5ed07fa54e4702c9d6c4174f74427fc0ef6e214bbd677ae50a5099946470` |
| `anymap2` | `0.13.0` | `MIT/Apache-2.0` | `d301b3b94cb4b2f23d7917810addbbaff90738e0ca2be692bd027e70d7e0330c` |
| `bit-set` | `0.5.3` | `MIT/Apache-2.0` | `0700ddab506f33b20a03b13996eccd309a48e5ff77d0d95926aa0210fb4e95f1` |
| `bit-vec` | `0.6.3` | `MIT/Apache-2.0` | `349f9b6a179ed607305526ca489b34ad0a41aed5f7980fa90eb03160b69598fb` |
| `const-random` | `0.1.18` | `MIT OR Apache-2.0` | `87e00182fe74b066627d63b85fd550ac2998d4b0bd86bfed477a0ae4c7c71359` |
| `const-random-macro` | `0.1.16` | `MIT OR Apache-2.0` | `f9d839f2a20b0aee515dc581a6172f2321f96cab76c1a38a4c584a194955390e` |
| `derive-new` | `0.5.9` | `MIT` | `3418329ca0ad70234b9735dc4ceed10af4df60eff9c8e7b06cb5e520d92c3535` |
| `dlv-list` | `0.5.2` | `MIT OR Apache-2.0` | `442039f5147480ba31067cb00ada1adae6892028e40e45fc5de7b7df6dcc1b5f` |
| `dyn-clone` | `1.0.20` | `MIT OR Apache-2.0` | `d0881ea181b1df73ff77ffaaf9c7544ecc11e82fba9b5f27b262a3c73a332555` |
| `dyn-hash` | `0.2.2` | `MIT OR Apache-2.0` | `15401da73a9ed8c80e3b2d4dc05fe10e7b72d7243b9f614e516a44fa99986e88` |
| `filetime` | `0.2.29` | `MIT/Apache-2.0` | `5c287a33c7f0a620c38e641e7f60827713987b3c0f26e8ddc9462cc69cf75759` |
| `hashbrown` | `0.14.5` | `MIT OR Apache-2.0` | `e5274423e17b7c9fc20b6e7e208532f9b19825d82dfd615708b70edd83df41f1` |
| `itertools` | `0.10.5` | `MIT/Apache-2.0` | `b0fd2260e829bddf4cb6ea802289de2f86d6a7a690192fbe91b3f46e0f2c8473` |
| `itertools` | `0.12.1` | `MIT OR Apache-2.0` | `ba291022dbbd398a455acf126c1e341954079855bc60dfdda641363bd6922569` |
| `itertools` | `0.14.0` | `MIT OR Apache-2.0` | `2b192c782037fadd9cfa75548310488aabdbf3d2da73885b31bd0abd03351285` |
| `liquid` | `0.26.11` | `MIT OR Apache-2.0` | `2a494c3f9dad3cb7ed16f1c51812cbe4b29493d6c2e5cd1e2b87477263d9534d` |
| `liquid-core` | `0.26.11` | `MIT OR Apache-2.0` | `fc623edee8a618b4543e8e8505584f4847a4e51b805db1af6d9af0a3395d0d57` |
| `liquid-derive` | `0.26.10` | `MIT OR Apache-2.0` | `de66c928222984aea59fcaed8ba627f388aaac3c1f57dcb05cc25495ef8faefe` |
| `liquid-lib` | `0.26.11` | `MIT OR Apache-2.0` | `9befeedd61f5995bc128c571db65300aeb50d62e4f0542c88282dbcb5f72372a` |
| `maplit` | `1.0.2` | `MIT/Apache-2.0` | `3e2e65a1a2e43cfcb47a895c4c8b10d1f4a61097f9f254f183aee60cad9c651d` |
| `matrixmultiply` | `0.3.11` | `MIT/Apache-2.0` | `3f607c237553f086e7043417a51df26b2eb899d3caff94e6a67592ff992fedc7` |
| `ndarray` | `0.16.1` | `MIT OR Apache-2.0` | `882ed72dce9365842bf196bdeedf5055305f11fc8c03dee7bb0194a6cad34841` |
| `nom-language` | `0.1.0` | `MIT` | `2de2bc5b451bfedaef92c90b8939a8fff5770bdcc1fafd6239d086aab8fa6b29` |
| `ordered-multimap` | `0.7.3` | `MIT` | `49203cdcae0030493bad186b28da2fa25645fa276a51b6fec8010d281e02ef79` |
| `pastey` | `0.1.1` | `MIT OR Apache-2.0` | `35fb2e5f958ec131621fdd531e9fc186ed768cbe395337403ae56c17a74c68ec` |
| `pest` | `2.9.2` | `MIT OR Apache-2.0` | `45d3aca230fad2e6f6317ca0a72724338c4960cb97168a85cdee66df4a9a21a8` |
| `pest_derive` | `2.9.2` | `MIT OR Apache-2.0` | `284b60557f2c4a2e72ad3f2d34d42685a2fa4a6a61d0d2a10c0ae2a5e916c2cf` |
| `pest_generator` | `2.9.2` | `MIT OR Apache-2.0` | `1d9d1f08a115309ee99268cf85e5228e0e56aa9caf8841ec12866b6be07c3109` |
| `pest_meta` | `2.9.2` | `MIT OR Apache-2.0` | `ed93ba1a9ffcca32130a5188701c81c0c49cf00d4b7c5007d5148951d743adcb` |
| `prost` | `0.11.9` | `Apache-2.0` | `0b82eaa1d779e9a4bc1c3217db8ffbeabaae1dca241bf70183242128d48681cd` |
| `prost-derive` | `0.11.9` | `Apache-2.0` | `e5d2d8d10f3c6ded6da8b05b5fb3b8a5082514344d56c9f871412d29b4e075b4` |
| `rand_distr` | `0.4.3` | `MIT OR Apache-2.0` | `32cb0b9bc82b0a0876c2dd994a7e7a2683d3e7390ca40e6886785ef0c7e3ee31` |
| `rawpointer` | `0.2.1` | `MIT/Apache-2.0` | `60a357793950651c4ed0f3f52338f53b2f809f32d83a07f72909fa13e4c6c1e3` |
| `rust-ini` | `0.21.3` | `MIT` | `796e8d2b6696392a43bea58116b667fb4c29727dc5abd27d6acf338bb4f688c7` |
| `safetensors` | `0.6.2` | `Apache-2.0` | `172dd94c5a87b5c79f945c863da53b2ebc7ccef4eca24ac63cca66a41aab2178` |
| `scan_fmt` | `0.2.6` | `MIT` | `0b53b0a5db882a8e2fdaae0a43f7b39e7e9082389e978398bdf223a55b581248` |
| `string-interner` | `0.15.0` | `MIT/Apache-2.0` | `07f9fdfdd31a0ff38b59deb401be81b73913d76c9cc5b1aed4e1330a223420b9` |
| `syn` | `1.0.109` | `MIT OR Apache-2.0` | `72b64191b275b66ffe2469e8af2c1cfe3bafa67b529ead792a6d0160888b4237` |
| `tar` | `0.4.46` | `MIT OR Apache-2.0` | `3f6221d9a6003c78398e3b239969f352578258df48c8eb051caadae0015bc840` |
| `tiny-keccak` | `2.0.2` | `CC0-1.0` | `2c9d3793400a45f954c52e73d068316d76b6f4e36977e3fcebb13a2721e80237` |
| `tract-core` | `0.22.4` | `MIT OR Apache-2.0` | `0509530e580e4b6695d8b1e254197640ebd15b51d30ed098e11a021f901eac28` |
| `tract-data` | `0.22.4` | `MIT OR Apache-2.0` | `29c9dbbf8dc971710999e8d4085dd7824886924f76dde88a1299cef64d8285c0` |
| `tract-hir` | `0.22.4` | `MIT OR Apache-2.0` | `ad86ddf6796f0022e61491f90c1fbd16713d6daa11fd8036e1a4322c9a56ba97` |
| `tract-linalg` | `0.22.4` | `MIT OR Apache-2.0` | `336792e9f5f3afe1b4ac9048103302ed250dd2e91e41ef06e243c863a1b5070a` |
| `tract-nnef` | `0.22.4` | `MIT OR Apache-2.0` | `c263022f047e8ba663d383038b19a8335421e7eb01335abf69e774638652f9e6` |
| `tract-onnx` | `0.22.4` | `MIT OR Apache-2.0` | `aee880962c55bb52c5b71ce28c0e83d6b3439a220efe93184690add9e759bfa0` |
| `tract-onnx-opl` | `0.22.4` | `MIT OR Apache-2.0` | `62ac7471be8a51308fc891999428103cf158e3b1834ebe634b2b656ea11fea0e` |
| `tract-pulse` | `0.22.4` | `MIT OR Apache-2.0` | `47affdfa009790eced303a114823102ac5a375f6095bd5713efd83128dea0e7d` |
| `tract-pulse-opl` | `0.22.4` | `MIT OR Apache-2.0` | `050ec344ec4acc5ca72732400da0eefd688ec4b1c84460ff24e6563c46ada556` |
| `ucd-trie` | `0.1.7` | `MIT OR Apache-2.0` | `2896d95c02a80c6d6a5d6e953d479f5ddf2dfdb6a244441010e373ac0fb88971` |
| `unicode-normalization` | `0.1.25` | `MIT OR Apache-2.0` | `5fd4f6878c9cb28d874b009da9e8d183b5abc80117c40bbd187a1fde336be6e8` |
| `xattr` | `1.6.1` | `MIT OR Apache-2.0` | `32e45ad4206f6d2479085147f02bc2ef834ac85886624a23575ae137c8aa8156` |

### Retained license and notice files

| File | Exact source | SHA-256 |
| --- | --- | --- |
| `anyhow-1.0.104-LICENSE-APACHE` | [registry source](https://docs.rs/crate/anyhow/1.0.104/source/LICENSE-APACHE) | `62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a` |
| `anyhow-1.0.104-LICENSE-MIT` | [registry source](https://docs.rs/crate/anyhow/1.0.104/source/LICENSE-MIT) | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| `anymap2-0.13.0-COPYRIGHT` | [registry source](https://docs.rs/crate/anymap2/0.13.0/source/COPYRIGHT) | `edb9885b41e4851740dcdcde4d2419c60104e1f3473f7bea6293a82729a32068` |
| `anymap2-0.13.0-LICENSE-APACHE` | [registry source](https://docs.rs/crate/anymap2/0.13.0/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `anymap2-0.13.0-LICENSE-MIT` | [registry source](https://docs.rs/crate/anymap2/0.13.0/source/LICENSE-MIT) | `b85cb7b51c3f8d600bbac3fb8dbbfe64cde697e460a55fc80978eb0804ca1427` |
| `bit-set-0.5.3-LICENSE-APACHE` | [registry source](https://docs.rs/crate/bit-set/0.5.3/source/LICENSE-APACHE) | `8173d5c29b4f956d532781d2b86e4e30f83e6b7878dce18c919451d6ba707c90` |
| `bit-set-0.5.3-LICENSE-MIT` | [registry source](https://docs.rs/crate/bit-set/0.5.3/source/LICENSE-MIT) | `c9a75f18b9ab2927829a208fc6aa2cf4e63b8420887ba29cdb265d6619ae82d5` |
| `bit-vec-0.6.3-LICENSE-APACHE` | [registry source](https://docs.rs/crate/bit-vec/0.6.3/source/LICENSE-APACHE) | `8173d5c29b4f956d532781d2b86e4e30f83e6b7878dce18c919451d6ba707c90` |
| `bit-vec-0.6.3-LICENSE-MIT` | [registry source](https://docs.rs/crate/bit-vec/0.6.3/source/LICENSE-MIT) | `7b63ecd5f1902af1b63729947373683c32745c16a10e8e6292e2e2dcd7e90ae0` |
| `const-random-0.1.18-LICENSE-APACHE` | [registry source](https://docs.rs/crate/const-random/0.1.18/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `const-random-0.1.18-LICENSE-MIT` | [registry source](https://docs.rs/crate/const-random/0.1.18/source/LICENSE-MIT) | `ff8f68cb076caf8cefe7a6430d4ac086ce6af2ca8ce2c4e5a2004d4552ef52a2` |
| `const-random-macro-0.1.16-LICENSE-APACHE` | [registry source](https://docs.rs/crate/const-random-macro/0.1.16/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `const-random-macro-0.1.16-LICENSE-MIT` | [registry source](https://docs.rs/crate/const-random-macro/0.1.16/source/LICENSE-MIT) | `ff8f68cb076caf8cefe7a6430d4ac086ce6af2ca8ce2c4e5a2004d4552ef52a2` |
| `derive-new-0.5.9-LICENSE` | [registry source](https://docs.rs/crate/derive-new/0.5.9/source/LICENSE) | `e13217b08deef741d527bd01178f8c2c801122fe9c3221f61762e95d31eceec1` |
| `dlv-list-0.5.2-LICENSE-APACHE` | [registry source](https://docs.rs/crate/dlv-list/0.5.2/source/LICENSE-APACHE) | `95bd3988beee069fa2848f648dab43cc6e0b2add2ad6bcb17360caf749802bcc` |
| `dlv-list-0.5.2-LICENSE-MIT` | [registry source](https://docs.rs/crate/dlv-list/0.5.2/source/LICENSE-MIT) | `77adcddfe9e50acd2df63ddbb8e566bb8d34b4d02a9d92e7a5c1b9c2225eda9f` |
| `dyn-clone-1.0.20-LICENSE-APACHE` | [registry source](https://docs.rs/crate/dyn-clone/1.0.20/source/LICENSE-APACHE) | `62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a` |
| `dyn-clone-1.0.20-LICENSE-MIT` | [registry source](https://docs.rs/crate/dyn-clone/1.0.20/source/LICENSE-MIT) | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| `dyn-hash-0.2.2-LICENSE-APACHE` | [registry source](https://docs.rs/crate/dyn-hash/0.2.2/source/LICENSE-APACHE) | `62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a` |
| `dyn-hash-0.2.2-LICENSE-MIT` | [registry source](https://docs.rs/crate/dyn-hash/0.2.2/source/LICENSE-MIT) | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| `filetime-0.2.29-LICENSE-APACHE` | [registry source](https://docs.rs/crate/filetime/0.2.29/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `filetime-0.2.29-LICENSE-MIT` | [registry source](https://docs.rs/crate/filetime/0.2.29/source/LICENSE-MIT) | `378f5840b258e2779c39418f3f2d7b2ba96f1c7917dd6be0713f88305dbda397` |
| `hashbrown-0.14.5-LICENSE-APACHE` | [registry source](https://docs.rs/crate/hashbrown/0.14.5/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `hashbrown-0.14.5-LICENSE-MIT` | [registry source](https://docs.rs/crate/hashbrown/0.14.5/source/LICENSE-MIT) | `ff8f68cb076caf8cefe7a6430d4ac086ce6af2ca8ce2c4e5a2004d4552ef52a2` |
| `itertools-0.10.5-LICENSE-APACHE` | [registry source](https://docs.rs/crate/itertools/0.10.5/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `itertools-0.10.5-LICENSE-MIT` | [registry source](https://docs.rs/crate/itertools/0.10.5/source/LICENSE-MIT) | `7576269ea71f767b99297934c0b2367532690f8c4badc695edf8e04ab6a1e545` |
| `itertools-0.12.1-LICENSE-APACHE` | [registry source](https://docs.rs/crate/itertools/0.12.1/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `itertools-0.12.1-LICENSE-MIT` | [registry source](https://docs.rs/crate/itertools/0.12.1/source/LICENSE-MIT) | `7576269ea71f767b99297934c0b2367532690f8c4badc695edf8e04ab6a1e545` |
| `itertools-0.14.0-LICENSE-APACHE` | [registry source](https://docs.rs/crate/itertools/0.14.0/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `itertools-0.14.0-LICENSE-MIT` | [registry source](https://docs.rs/crate/itertools/0.14.0/source/LICENSE-MIT) | `7576269ea71f767b99297934c0b2367532690f8c4badc695edf8e04ab6a1e545` |
| `liquid-0.26.11-LICENSE` | [registry source](https://docs.rs/crate/liquid/0.26.11/source/LICENSE) | `7b4b09e856fad3b9d6e480a9ec8ccaf85080c5ad8e536aade8fe30b2c533a779` |
| `liquid-0.26.11-LICENSE-APACHE` | [registry source](https://docs.rs/crate/liquid/0.26.11/source/LICENSE-APACHE) | `c6596eb7be8581c18be736c846fb9173b69eccf6ef94c5135893ec56bd92ba08` |
| `liquid-0.26.11-LICENSE-MIT` | [registry source](https://docs.rs/crate/liquid/0.26.11/source/LICENSE-MIT) | `6efb0476a1cc085077ed49357026d8c173bf33017278ef440f222fb9cbcb66e6` |
| `liquid-core-0.26.11-LICENSE-APACHE` | [registry source](https://docs.rs/crate/liquid-core/0.26.11/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `liquid-core-0.26.11-LICENSE-MIT` | [registry source](https://docs.rs/crate/liquid-core/0.26.11/source/LICENSE-MIT) | `6a5dfb0adf37850239f4b2388a79355c77b625d6c5542dea7743ac5033efaba2` |
| `liquid-derive-0.26.10-LICENSE-APACHE` | [registry source](https://docs.rs/crate/liquid-derive/0.26.10/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `liquid-derive-0.26.10-LICENSE-MIT` | [registry source](https://docs.rs/crate/liquid-derive/0.26.10/source/LICENSE-MIT) | `6a5dfb0adf37850239f4b2388a79355c77b625d6c5542dea7743ac5033efaba2` |
| `liquid-lib-0.26.11-LICENSE` | [registry source](https://docs.rs/crate/liquid-lib/0.26.11/source/LICENSE) | `7b4b09e856fad3b9d6e480a9ec8ccaf85080c5ad8e536aade8fe30b2c533a779` |
| `maplit-1.0.2-LICENSE-APACHE` | [registry source](https://docs.rs/crate/maplit/1.0.2/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `maplit-1.0.2-LICENSE-MIT` | [registry source](https://docs.rs/crate/maplit/1.0.2/source/LICENSE-MIT) | `7576269ea71f767b99297934c0b2367532690f8c4badc695edf8e04ab6a1e545` |
| `matrixmultiply-0.3.11-LICENSE-APACHE` | [registry source](https://docs.rs/crate/matrixmultiply/0.3.11/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `matrixmultiply-0.3.11-LICENSE-MIT` | [registry source](https://docs.rs/crate/matrixmultiply/0.3.11/source/LICENSE-MIT) | `792d075c7bad6dac258a44e799eb64cbf465e24d9932d27669be08c5ec957e27` |
| `ndarray-0.16.1-LICENSE-APACHE` | [registry source](https://docs.rs/crate/ndarray/0.16.1/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `ndarray-0.16.1-LICENSE-MIT` | [registry source](https://docs.rs/crate/ndarray/0.16.1/source/LICENSE-MIT) | `1fd6747d2c8e80f9fa766f57c5888864774621deb85cc2838ccaed727db32d45` |
| `nom-language-0.1.0-LICENSE` | [pinned upstream](https://raw.githubusercontent.com/rust-bakery/nom/2cec1b3e4c9ccac62c902d60c00de6d1549ccbe1/LICENSE) | `4dbda04344456f09a7a588140455413a9ac59b6b26a1ef7cdf9c800c012d87f0` |
| `ordered-multimap-0.7.3-LICENSE` | [registry source](https://docs.rs/crate/ordered-multimap/0.7.3/source/LICENSE) | `047c1d2f1c28c30ced89bd0740ff251d8f51512e81b142711f958a0551729ec4` |
| `pastey-0.1.1-LICENSE-APACHE` | [registry source](https://docs.rs/crate/pastey/0.1.1/source/LICENSE-APACHE) | `62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a` |
| `pastey-0.1.1-LICENSE-MIT` | [registry source](https://docs.rs/crate/pastey/0.1.1/source/LICENSE-MIT) | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| `pest-2.9.2-LICENSE-APACHE` | [registry source](https://docs.rs/crate/pest/2.9.2/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `pest-2.9.2-LICENSE-MIT` | [registry source](https://docs.rs/crate/pest/2.9.2/source/LICENSE-MIT) | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| `pest_derive-2.9.2-LICENSE-APACHE` | [registry source](https://docs.rs/crate/pest_derive/2.9.2/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `pest_derive-2.9.2-LICENSE-MIT` | [registry source](https://docs.rs/crate/pest_derive/2.9.2/source/LICENSE-MIT) | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| `pest_generator-2.9.2-LICENSE-APACHE` | [registry source](https://docs.rs/crate/pest_generator/2.9.2/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `pest_generator-2.9.2-LICENSE-MIT` | [registry source](https://docs.rs/crate/pest_generator/2.9.2/source/LICENSE-MIT) | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| `pest_meta-2.9.2-LICENSE-APACHE` | [registry source](https://docs.rs/crate/pest_meta/2.9.2/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `pest_meta-2.9.2-LICENSE-MIT` | [registry source](https://docs.rs/crate/pest_meta/2.9.2/source/LICENSE-MIT) | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| `prost-0.11.9-LICENSE` | [registry source](https://docs.rs/crate/prost/0.11.9/source/LICENSE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `prost-derive-0.11.9-LICENSE` | [registry source](https://docs.rs/crate/prost-derive/0.11.9/source/LICENSE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `rand_distr-0.4.3-COPYRIGHT` | [registry source](https://docs.rs/crate/rand_distr/0.4.3/source/COPYRIGHT) | `90eb64f0279b0d9432accfa6023ff803bc4965212383697eee27a0f426d5f8d5` |
| `rand_distr-0.4.3-LICENSE-APACHE` | [registry source](https://docs.rs/crate/rand_distr/0.4.3/source/LICENSE-APACHE) | `6df43f6f4b5d4587f3d8d71e45532c688fd168afa5fe89d571cb32fa09c4ef51` |
| `rand_distr-0.4.3-LICENSE-MIT` | [registry source](https://docs.rs/crate/rand_distr/0.4.3/source/LICENSE-MIT) | `a771e4354f6b3ad4c92da1a5c9a239b6c291527db869632ecea4f20e24ca1135` |
| `rawpointer-0.2.1-LICENSE-APACHE` | [registry source](https://docs.rs/crate/rawpointer/0.2.1/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `rawpointer-0.2.1-LICENSE-MIT` | [registry source](https://docs.rs/crate/rawpointer/0.2.1/source/LICENSE-MIT) | `7576269ea71f767b99297934c0b2367532690f8c4badc695edf8e04ab6a1e545` |
| `rust-ini-0.21.3-LICENSE` | [registry source](https://docs.rs/crate/rust-ini/0.21.3/source/LICENSE) | `ccf6244964385d34fef3799aa7792e9f8d35517de026f39a4f43f0e89b2079eb` |
| `safetensors-0.6.2-LICENSE` | [registry source](https://docs.rs/crate/safetensors/0.6.2/source/LICENSE) | `c71d239df91726fc519c6eb72d318ec65820627232b2f796219e87dcf35d0ab4` |
| `scan_fmt-0.2.6-LICENSE` | [registry source](https://docs.rs/crate/scan_fmt/0.2.6/source/LICENSE) | `4d0814fe61e6458a52ce8c744f23c6e7fbdacd20f4f6452bc5f29b7790a98588` |
| `string-interner-0.15.0-LICENSE-APACHE` | [registry source](https://docs.rs/crate/string-interner/0.15.0/source/LICENSE-APACHE) | `1129d26272a056e37e5e7bd2985b8c72e6acd8f55c899c7a2bc136d13ee172b6` |
| `string-interner-0.15.0-LICENSE-MIT` | [registry source](https://docs.rs/crate/string-interner/0.15.0/source/LICENSE-MIT) | `7f304506d28acecccba0e2ee0bb6aebdf46385d4a7cd602e18dd1a2f123bba75` |
| `syn-1.0.109-LICENSE-APACHE` | [registry source](https://docs.rs/crate/syn/1.0.109/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `syn-1.0.109-LICENSE-MIT` | [registry source](https://docs.rs/crate/syn/1.0.109/source/LICENSE-MIT) | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| `tar-0.4.46-LICENSE-APACHE` | [registry source](https://docs.rs/crate/tar/0.4.46/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `tar-0.4.46-LICENSE-MIT` | [registry source](https://docs.rs/crate/tar/0.4.46/source/LICENSE-MIT) | `8ca6b96cea9e67c6c5c63f452c31bd396db8bd2406231fdea5d48ef462b48077` |
| `tiny-keccak-2.0.2-LICENSE` | [registry source](https://docs.rs/crate/tiny-keccak/2.0.2/source/LICENSE) | `a2010f343487d3f7618affe54f789f5487602331c0a8d03f49e9a7c547cf0499` |
| `tract-core-0.22.4-LICENSE` | [registry source](https://docs.rs/crate/tract-core/0.22.4/source/LICENSE) | `f7ef673bf046d823dcd775bdd0768432bd8855f81d0e5e1290a0a48c42e2dca3` |
| `tract-core-0.22.4-LICENSE-APACHE` | [registry source](https://docs.rs/crate/tract-core/0.22.4/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `tract-core-0.22.4-LICENSE-MIT` | [registry source](https://docs.rs/crate/tract-core/0.22.4/source/LICENSE-MIT) | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| `tract-data-0.22.4-LICENSE` | [registry source](https://docs.rs/crate/tract-data/0.22.4/source/LICENSE) | `f7ef673bf046d823dcd775bdd0768432bd8855f81d0e5e1290a0a48c42e2dca3` |
| `tract-data-0.22.4-LICENSE-APACHE` | [registry source](https://docs.rs/crate/tract-data/0.22.4/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `tract-data-0.22.4-LICENSE-MIT` | [registry source](https://docs.rs/crate/tract-data/0.22.4/source/LICENSE-MIT) | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| `tract-hir-0.22.4-LICENSE` | [registry source](https://docs.rs/crate/tract-hir/0.22.4/source/LICENSE) | `f7ef673bf046d823dcd775bdd0768432bd8855f81d0e5e1290a0a48c42e2dca3` |
| `tract-hir-0.22.4-LICENSE-APACHE` | [registry source](https://docs.rs/crate/tract-hir/0.22.4/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `tract-hir-0.22.4-LICENSE-MIT` | [registry source](https://docs.rs/crate/tract-hir/0.22.4/source/LICENSE-MIT) | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| `tract-linalg-0.22.4-LICENSE` | [registry source](https://docs.rs/crate/tract-linalg/0.22.4/source/LICENSE) | `f7ef673bf046d823dcd775bdd0768432bd8855f81d0e5e1290a0a48c42e2dca3` |
| `tract-linalg-0.22.4-LICENSE-APACHE` | [registry source](https://docs.rs/crate/tract-linalg/0.22.4/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `tract-linalg-0.22.4-LICENSE-MIT` | [registry source](https://docs.rs/crate/tract-linalg/0.22.4/source/LICENSE-MIT) | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| `tract-nnef-0.22.4-LICENSE` | [registry source](https://docs.rs/crate/tract-nnef/0.22.4/source/LICENSE) | `f7ef673bf046d823dcd775bdd0768432bd8855f81d0e5e1290a0a48c42e2dca3` |
| `tract-nnef-0.22.4-LICENSE-APACHE` | [registry source](https://docs.rs/crate/tract-nnef/0.22.4/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `tract-nnef-0.22.4-LICENSE-MIT` | [registry source](https://docs.rs/crate/tract-nnef/0.22.4/source/LICENSE-MIT) | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| `tract-onnx-0.22.4-LICENSE` | [registry source](https://docs.rs/crate/tract-onnx/0.22.4/source/LICENSE) | `f7ef673bf046d823dcd775bdd0768432bd8855f81d0e5e1290a0a48c42e2dca3` |
| `tract-onnx-0.22.4-LICENSE-APACHE` | [registry source](https://docs.rs/crate/tract-onnx/0.22.4/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `tract-onnx-0.22.4-LICENSE-MIT` | [registry source](https://docs.rs/crate/tract-onnx/0.22.4/source/LICENSE-MIT) | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| `tract-onnx-opl-0.22.4-LICENSE` | [registry source](https://docs.rs/crate/tract-onnx-opl/0.22.4/source/LICENSE) | `f7ef673bf046d823dcd775bdd0768432bd8855f81d0e5e1290a0a48c42e2dca3` |
| `tract-onnx-opl-0.22.4-LICENSE-APACHE` | [registry source](https://docs.rs/crate/tract-onnx-opl/0.22.4/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `tract-onnx-opl-0.22.4-LICENSE-MIT` | [registry source](https://docs.rs/crate/tract-onnx-opl/0.22.4/source/LICENSE-MIT) | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| `tract-pulse-0.22.4-LICENSE` | [registry source](https://docs.rs/crate/tract-pulse/0.22.4/source/LICENSE) | `f7ef673bf046d823dcd775bdd0768432bd8855f81d0e5e1290a0a48c42e2dca3` |
| `tract-pulse-0.22.4-LICENSE-APACHE` | [registry source](https://docs.rs/crate/tract-pulse/0.22.4/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `tract-pulse-0.22.4-LICENSE-MIT` | [registry source](https://docs.rs/crate/tract-pulse/0.22.4/source/LICENSE-MIT) | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| `tract-pulse-opl-0.22.4-LICENSE` | [registry source](https://docs.rs/crate/tract-pulse-opl/0.22.4/source/LICENSE) | `f7ef673bf046d823dcd775bdd0768432bd8855f81d0e5e1290a0a48c42e2dca3` |
| `tract-pulse-opl-0.22.4-LICENSE-APACHE` | [registry source](https://docs.rs/crate/tract-pulse-opl/0.22.4/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `tract-pulse-opl-0.22.4-LICENSE-MIT` | [registry source](https://docs.rs/crate/tract-pulse-opl/0.22.4/source/LICENSE-MIT) | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| `ucd-trie-0.1.7-LICENSE-APACHE` | [registry source](https://docs.rs/crate/ucd-trie/0.1.7/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `ucd-trie-0.1.7-LICENSE-MIT` | [registry source](https://docs.rs/crate/ucd-trie/0.1.7/source/LICENSE-MIT) | `0f96a83840e146e43c0ec96a22ec1f392e0680e6c1226e6f3ba87e0740af850f` |
| `unicode-normalization-0.1.25-COPYRIGHT` | [registry source](https://docs.rs/crate/unicode-normalization/0.1.25/source/COPYRIGHT) | `23860c2a7b5d96b21569afedf033469bab9fe14a1b24a35068b8641c578ce24d` |
| `unicode-normalization-0.1.25-LICENSE-APACHE` | [registry source](https://docs.rs/crate/unicode-normalization/0.1.25/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `unicode-normalization-0.1.25-LICENSE-MIT` | [registry source](https://docs.rs/crate/unicode-normalization/0.1.25/source/LICENSE-MIT) | `7b63ecd5f1902af1b63729947373683c32745c16a10e8e6292e2e2dcd7e90ae0` |
| `xattr-1.6.1-LICENSE-APACHE` | [registry source](https://docs.rs/crate/xattr/1.6.1/source/LICENSE-APACHE) | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `xattr-1.6.1-LICENSE-MIT` | [registry source](https://docs.rs/crate/xattr/1.6.1/source/LICENSE-MIT) | `8b427f5bc501764575e52ba4f9d95673cf8f6d80a86d0d06599852e1a9a20a36` |
