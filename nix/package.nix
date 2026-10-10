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

    # Reuse the same small, LGPL-only recipe as official native packages. Never
    # link the default nixpkgs FFmpeg, which may enable GPL components.
    sereinFfmpeg = stdenv.mkDerivation {
        pname = "serein-ffmpeg";
        version = "7.1.5";
        dontUnpack = true;
        nativeBuildInputs = [python3 pkg-config nasm]
            ++ lib.optionals isLinux [cmake patchelf]
            ++ lib.optionals isDarwin [darwin.sigtool cctools];
        CC = "${stdenv.cc}/bin/cc";
        CXX = "${stdenv.cc}/bin/c++";
        buildInputs = lib.optionals isDarwin [apple-sdk_15]
            ++ lib.optionals isLinux [libdrm]
            ++ lib.optionals (isLinux && stdenv.hostPlatform.isx86_64) [libva];
        dontUseCmakeConfigure = true;
        installPhase = ''
            runHook preInstall
            mkdir sources
            ${lib.concatMapStringsSep "\n" (source: ''ln -s ${source} sources/${source.name}'') ffmpegSources}
            python3 ${../scripts/build-ffmpeg.py} --prefix "$out" --work-dir build \
              --cache-dir sources --jobs "$NIX_BUILD_CORES" --offline
            runHook postInstall
        '';
        meta.license = [lib.licenses.lgpl21Plus lib.licenses.bsd2 lib.licenses.mit];
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
            lockFile = "${finalAttrs.src}/Cargo.lock";
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
        '';

        preFixup = lib.optionalString isLinux ''
            gappsWrapperArgs+=(
              --prefix PATH : "${lib.makeBinPath runtimeTools}"
              --prefix GST_PLUGIN_SYSTEM_PATH_1_0 : "${
                lib.makeSearchPathOutput "lib" "lib/gstreamer-1.0" gstPlugins
            }"
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
                cp -R ${sereinFfmpeg}/share/serein-ffmpeg "$docs/ffmpeg-source"
            ''
            + lib.optionalString isLinux ''
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
                install -Dm444 packaging/macos/Info.plist "$app/Info.plist"
                install -Dm444 packaging/macos/Serein.icns "$app/Resources/Serein.icns"
                mkdir -p "$app/MacOS" "$out/share/doc"
                mv "$out/bin/serein" "$app/MacOS/serein"
                ln -s "$app/MacOS/serein" "$out/bin/serein"
                ln -s "$app/Resources" "$out/share/doc/serein"
            '';

        meta = {
            description = "Tiny, performant, native Discord client written in Rust (egui/wgpu)";
            homepage = "https://github.com/ViceVerse-cz/Serein";
            license = with lib.licenses; [
                mit
                asl20
            ];
            platforms = lib.platforms.linux ++ lib.platforms.darwin;
            mainProgram = "serein";
            maintainers = with lib.maintainers; [
                myamusashi
            ];
        };
    })
