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

## Vendored epaint RTL metadata (October 2, 2026)

`vendor/epaint` copies epaint 0.36.2 from egui revision
`fe6d63efa4a4df6f56ceab814d4f3a6efab69b88`; its opt-in direction and exact UTF-8
cluster metadata changes are recorded in `epaint-SEREIN-PATCH.md`, copied into
this bundled notice directory. The upstream MIT and Apache texts at that revision
are byte-identical to the existing `egui-LICENSE-MIT` and `egui-LICENSE-APACHE`
files above. The retained source manifest, original manifest, licenses and patch
notice remain in `vendor/epaint`. Native shaping and installed/bundled font
identities are unchanged.
