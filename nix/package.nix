{
    lib,
    stdenv,
    rustPlatform,
    pkg-config,
    cmake,
    cctools,
    darwin,
    makeWrapper,
    swift,
    swiftpm,
    swiftPackages,
    apple-sdk_15,
    wrapGAppsHook4,
    autoPatchelfHook,
    glib,
    glib-networking,
    gsettings-desktop-schemas,
    gtk4,
    webkitgtk_6_0,
    cairo,
    pango,
    gdk-pixbuf,
    graphene,
    libsoup_3,
    fontconfig,
    libxkbcommon,
    wayland,
    vulkan-loader,
    libGL,
    libGLX,
    libpulseaudio,
    libglvnd,
    alsa-lib,
    gst_all_1,
    pipewire,
    libX11,
    libXi,
    libXrandr,
    libXcursor,
    bubblewrap,
    xdg-dbus-proxy,
    fetchurl,
    python3,
    nasm,
    meson,
    ninja,
    patchelf,
    libva,
    libdrm,
}: let
    inherit (stdenv.hostPlatform) isLinux isDarwin;

    ffmpegSources = [
        (fetchurl {
            url = "https://ffmpeg.org/releases/ffmpeg-7.1.5.tar.xz";
            name = "ffmpeg-7.1.5.tar.xz";
            sha256 = "de668509caf9e35e3cd162473441fdb29538c6d96ed080292b3cf9e6fc5d558f";
        })
        (fetchurl {
            url = "https://codeload.github.com/cisco/openh264/tar.gz/refs/tags/v2.6.0";
            name = "openh264-2.6.0.tar.gz";
            sha256 = "558544ad358283a7ab2930d69a9ceddf913f4a51ee9bf1bfb9e377322af81a69";
        })
        (fetchurl {
            url = "https://downloads.videolan.org/pub/videolan/dav1d/1.5.1/dav1d-1.5.1.tar.xz";
            name = "dav1d-1.5.1.tar.xz";
            sha256 = "401813f1f89fa8fd4295805aa5284d9aed9bc7fc1fdbe554af4292f64cbabe21";
        })
    ] ++ lib.optionals isLinux [
        (fetchurl {
            url = "https://codeload.github.com/KhronosGroup/Vulkan-Headers/tar.gz/refs/tags/v1.3.290";
            name = "Vulkan-Headers-1.3.290.tar.gz";
            sha256 = "f38a653bf93cab7a2a229a53d2d53b1cba9a2819e4c0a7de13c54085bde9bcf5";
        })
        (fetchurl {
            url = "https://codeload.github.com/FFmpeg/nv-codec-headers/tar.gz/refs/tags/n12.2.72.0";
            name = "nv-codec-headers-12.2.72.0.tar.gz";
            sha256 = "dbeaec433d93b850714760282f1d0992b1254fc3b5a6cb7d76fc1340a1e47563";
        })
        (fetchurl {
            url = "https://codeload.github.com/GPUOpen-LibrariesAndSDKs/AMF/tar.gz/refs/tags/v1.4.36";
            name = "AMF-1.4.36.tar.gz";
            sha256 = "240a42033babc7920e5476506d5ac0c5628f67908833168e746406808d0ef146";
        })
    ] ++ lib.optionals (isLinux && stdenv.hostPlatform.isx86_64) [
        (fetchurl {
            url = "https://codeload.github.com/intel/libvpl/tar.gz/refs/tags/v2.14.0";
            name = "libvpl-2.14.0.tar.gz";
            sha256 = "7c6bff1c1708d910032c2e6c44998ffff3f5fdbf06b00972bc48bf2dd9e5ac06";
        })
    ];

    # Reuse the same pinned recipe as official native packages. Do not link
    # the default nixpkgs FFmpeg, whose enabled components can differ.
    sereinFfmpeg = stdenv.mkDerivation {
        pname = "serein-ffmpeg";
        version = "7.1.5";
        dontUnpack = true;
        nativeBuildInputs = [python3 pkg-config nasm meson ninja]
            ++ lib.optionals isLinux [cmake patchelf]
            ++ lib.optionals isDarwin [darwin.sigtool cctools];
        CC = "${stdenv.cc}/bin/cc";
        CXX = "${stdenv.cc}/bin/c++";
        buildInputs = lib.optionals isDarwin [apple-sdk_15]
            ++ lib.optionals isLinux [libdrm]
            ++ lib.optionals (isLinux && stdenv.hostPlatform.isx86_64) [libva];
        dontUseCmakeConfigure = true;
        dontUseMesonConfigure = true;
        # The shared Python recipe configures and builds each native dependency below.
        dontBuild = true;
        installPhase = ''
            runHook preInstall
            mkdir sources
            ${lib.concatMapStringsSep "\n" (source: ''ln -s ${source} sources/${source.name}'') ffmpegSources}
            python3 ${../scripts/build-ffmpeg.py} --prefix "$out" --work-dir build \
              --cache-dir sources --jobs "$NIX_BUILD_CORES" --offline
            runHook postInstall
        '';
        meta.license = [lib.licenses.gpl2Plus lib.licenses.lgpl21Plus lib.licenses.bsd2 lib.licenses.mit];
    };

    speechSources = [
        (fetchurl {
            url = "https://github.com/idocinthebox/LavaSR-ONNX-Embedded/releases/download/v1.0.0/backbone.onnx";
            name = "backbone.onnx";
            sha256 = "0bc915649f765d40ca52bd3a2272a28e8f7636b09c2ee3148662a69041f28cd9";
        })
        (fetchurl {
            url = "https://github.com/idocinthebox/LavaSR-ONNX-Embedded/releases/download/v1.0.0/spec_head.onnx";
            name = "spec_head.onnx";
            sha256 = "a54a5d5d652aceb76b2f48cf9f0ab3f4def0d44427dc7af641fd24284342a48a";
        })
        (fetchurl {
            url = "https://github.com/idocinthebox/LavaSR-ONNX-Embedded/releases/download/v1.0.0/denoiser_core.onnx";
            name = "denoiser_core.onnx";
            sha256 = "515a1be1eea4b1108cc6a758198d726fc506e68ec53d7e522bc2618ff2602175";
        })
        (fetchurl {
            url = "https://github.com/eigen-mirror/eigen/archive/1d8b82b0740839c0de7f1242a3585e3390ff5f33/eigen-1d8b82b0740839c0de7f1242a3585e3390ff5f33.zip";
            name = "eigen-1d8b82b0740839c0de7f1242a3585e3390ff5f33.zip";
            sha256 = "6a60d76351f97132669daeeb721d6bf14b008101883ad2d687a3201c5c461eb0";
        })
    ] ++ [ (fetchurl (if isDarwin && stdenv.hostPlatform.isAarch64 then {
        url = "https://github.com/microsoft/onnxruntime/releases/download/v1.23.2/onnxruntime-osx-arm64-1.23.2.tgz";
        name = "onnxruntime-osx-arm64-1.23.2.tgz";
        sha256 = "b4d513ab2b26f088c66891dbbc1408166708773d7cc4163de7bdca0e9bbb7856";
    } else if isDarwin then {
        url = "https://github.com/microsoft/onnxruntime/releases/download/v1.23.2/onnxruntime-osx-x86_64-1.23.2.tgz";
        name = "onnxruntime-osx-x86_64-1.23.2.tgz";
        sha256 = "d10359e16347b57d9959f7e80a225a5b4a66ed7d7e007274a15cae86836485a6";
    } else if stdenv.hostPlatform.isAarch64 then {
        url = "https://github.com/microsoft/onnxruntime/releases/download/v1.23.2/onnxruntime-linux-aarch64-1.23.2.tgz";
        name = "onnxruntime-linux-aarch64-1.23.2.tgz";
        sha256 = "7c63c73560ed76b1fac6cff8204ffe34fe180e70d6582b5332ec094810241e5c";
    } else {
        url = "https://github.com/microsoft/onnxruntime/releases/download/v1.23.2/onnxruntime-linux-x64-1.23.2.tgz";
        name = "onnxruntime-linux-x64-1.23.2.tgz";
        sha256 = "1fa4dcaef22f6f7d5cd81b28c2800414350c10116f5fdd46a2160082551c5f9b";
    })) ];

    sereinSpeech = stdenv.mkDerivation {
        pname = "serein-speech";
        version = "1.23.2";
        dontUnpack = true;
        dontBuild = true;
        nativeBuildInputs = [python3] ++ lib.optionals isLinux [autoPatchelfHook];
        buildInputs = lib.optionals isLinux [stdenv.cc.cc.lib];
        installPhase = ''
            mkdir speech-cache
            ${lib.concatMapStringsSep "\n" (source: ''cp ${source} speech-cache/${source.name}'') speechSources}
            python3 ${../packaging/lavasr/bundle.py} "$out" --cache-dir speech-cache --offline
        '';
        meta.license = [lib.licenses.mit lib.licenses.asl20 lib.licenses.mpl20];
    };

    # webKit stack
    toolkitDeps = [
        glib
        glib-networking
        gsettings-desktop-schemas
        gtk4
        webkitgtk_6_0
        cairo
        pango
        gdk-pixbuf
        graphene
        libsoup_3
        fontconfig
    ];

    graphicsDeps = [
        vulkan-loader
        libGL
        libGLX
        libglvnd
    ];

    windowingDeps = [
        wayland
        libxkbcommon
        libX11
        libXi
        libXrandr
        libXcursor
    ];

    audioDeps = [
        libpulseaudio
        alsa-lib
    ];

    gstPlugins = with gst_all_1; [
        gst-plugins-base
        gst-plugins-good
        gst-plugins-bad
        gst-libav
        gstreamer
        pipewire
    ];

    runtimeTools = [
        bubblewrap
        xdg-dbus-proxy
    ];
in
    rustPlatform.buildRustPackage (finalAttrs: {
        pname = "serein";
        # Match `cargo build`: release automation versions Cargo.toml, not this file.
        version = (lib.importTOML ../Cargo.toml).workspace.package.version;

        src = ../.;

        cargoLock = {
            lockFile = finalAttrs.src + "/Cargo.lock";
            allowBuiltinFetchGit = true;
        };

        cargoBuildFlags = [
            "--package"
            "serein"
        ];

        FFMPEG_DIR = "${sereinFfmpeg}";

        nativeBuildInputs =
            [
                pkg-config
                cmake
                makeWrapper
            ]
            ++ lib.optionals isLinux [
                wrapGAppsHook4
                autoPatchelfHook
            ]
            ++ lib.optionals isDarwin [
                swift
                swiftpm
            ];

        buildInputs =
            lib.optionals isLinux (
                toolkitDeps ++ graphicsDeps ++ windowingDeps ++ audioDeps ++ gstPlugins
            )
            ++ lib.optionals isDarwin [
                apple-sdk_15
                swiftPackages.stdlib
            ] ++ [sereinFfmpeg];

        # winit loads these libraries dynamically; retain them in the runtime RPATH.
        runtimeDependencies = lib.optionals isLinux (graphicsDeps ++ windowingDeps);

        dontUseSwiftpmBuild = true;
        dontUseSwiftpmCheck = true;
        dontUseSwiftpmInstall = true;
        doCheck = false;

        # Swift 6 split the stdlib out of the toolchain and SDK: the final Rust
        # link needs the SDK overlays plus the store stdlib/toolchain libs.
        preBuild = lib.optionalString isDarwin ''
            export RUSTFLAGS="$RUSTFLAGS -L native=$SDKROOT/usr/lib/swift -L native=${lib.getLib swiftPackages.stdlib}/lib -L native=${lib.getDev swiftPackages.stdlib}/lib -L native=${lib.getLib swift}/lib"
            # Clang's runtime provides the availability check used by capture.
            export NIX_LDFLAGS="$NIX_LDFLAGS -lclang_rt.osx"
        '';

        preFixup = lib.optionalString isLinux ''
            gappsWrapperArgs+=(
              --prefix PATH : "${lib.makeBinPath runtimeTools}"
              --prefix GST_PLUGIN_SYSTEM_PATH_1_0 : "${
                lib.makeSearchPathOutput "lib" "lib/gstreamer-1.0" gstPlugins
            }"
              --suffix LD_LIBRARY_PATH : /run/opengl-driver/lib
            )
        '';

        # Mirror `cargo xtask package`: notices, licenses and the corresponding
        # MPL-2.0 hpke-rs source ship with every binary.
        postInstall =
            ''
                docs="$out/share/doc/serein"
            ''
            + lib.optionalString isDarwin ''
                app="$out/Applications/Serein.app/Contents"
                docs="$app/Resources"
            ''
            + ''
                mkdir -p "$docs/licenses" "$docs/source"
                cp README.md LICENSE-MIT LICENSE-APACHE THIRD_PARTY_NOTICES.md "$docs/"
                cp -R assets/licenses/. "$docs/licenses/"
                cp assets/fonts/*-OFL.txt assets/fonts/*-LICENSE.txt "$docs/licenses/"
                cp assets/sounds/README.md "$docs/licenses/notification-sounds.md"
                cp assets/twemoji/LICENSE-GRAPHICS "$docs/licenses/Twemoji-CC-BY-4.0.txt"
                cp assets/twemoji/LICENSE-UNICODE "$docs/licenses/Unicode-LICENSE.txt"
                cp assets/icons/LICENSE "$docs/licenses/Phosphor-Icons-MIT.txt"
                cp assets/icons/LICENSE-SIMPLE-ICONS "$docs/licenses/Simple-Icons-CC0.txt"
                cp -R vendor/hpke-rs "$docs/source/"
                mkdir -p "$docs/ffmpeg-source/camera-filter"
                cp -R ${sereinFfmpeg}/share/serein-ffmpeg/. "$docs/ffmpeg-source/"
                cp -R crates/platform/src/video/denoise/. "$docs/ffmpeg-source/camera-filter/"
                cp crates/platform/src/video/denoise.rs "$docs/ffmpeg-source/camera-filter/denoise.rs"
            ''
            + lib.optionalString isLinux ''
                mkdir -p "$out/lib/serein"
                cp -R ${sereinSpeech}/voice-enhancement "$out/lib/serein/"
                install -Dm444 packaging/linux/serein.desktop \
                  $out/share/applications/cz.viceverse.serein.desktop
                substituteInPlace $out/share/applications/cz.viceverse.serein.desktop \
                  --replace-fail "Exec=serein" "Exec=$out/bin/serein"
                mkdir -p $out/share/icons
                cp -R packaging/linux/hicolor $out/share/icons/
            ''
            # The executable lives inside the bundle so macOS resolves its Info.plist
            # (privacy usage descriptions, identifier and icon) from the launched binary.
            + lib.optionalString isDarwin ''
                mkdir -p "$app/Resources" "$app/Frameworks"
                cp -R ${sereinSpeech}/Serein.app/Contents/Resources/voice-enhancement "$app/Resources/"
                cp ${sereinSpeech}/Serein.app/Contents/Frameworks/*.dylib "$app/Frameworks/"
                for library in \
                    libavcodec-serein.61.dylib libavutil-serein.59.dylib \
                    libavformat-serein.61.dylib libavfilter-serein.10.dylib \
                    libswscale-serein.8.dylib libswresample-serein.5.dylib \
                    libopenh264.8.dylib; do
                    install -m755 "${sereinFfmpeg}/lib/$library" "$app/Frameworks/$library"
                done
                install -Dm444 packaging/macos/Info.plist "$app/Info.plist"
                install -Dm444 packaging/macos/Serein.icns "$app/Resources/Serein.icns"
                mkdir -p "$app/MacOS" "$out/share/doc"
                mv "$out/bin/serein" "$app/MacOS/serein"
                ln -s "$app/MacOS/serein" "$out/bin/serein"
                ln -s "$app/Resources" "$out/share/doc/serein"
            '';

        meta = {
            description = "Tiny, performant, native Discord client written in Rust (egui/wgpu)";
            homepage = "https://github.com/MCShotty/NuSerein";
            license = with lib.licenses; [
                mit
                asl20
                gpl2Plus
            ];
            platforms = lib.platforms.linux ++ lib.platforms.darwin;
            mainProgram = "serein";
            maintainers = with lib.maintainers; [
                myamusashi
            ];
        };
    })
