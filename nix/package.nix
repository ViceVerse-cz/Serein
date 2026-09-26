{
    lib,
    stdenv,
    rustPlatform,
    pkg-config,
    cmake,
    makeWrapper,
    swift,
    swiftpm,
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
}: let
    inherit (stdenv.hostPlatform) isLinux isDarwin;

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
        version = (builtins.fromTOML (builtins.readFile ../Cargo.toml)).workspace.package.version;

        src = ../.;

        cargoLock = {
            lockFile = "${finalAttrs.src}/Cargo.lock";
            allowBuiltinFetchGit = true;
        };

        cargoBuildFlags = [
            "--package"
            "serein"
        ];

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

        buildInputs = lib.optionals isLinux (
            toolkitDeps ++ graphicsDeps ++ windowingDeps ++ audioDeps ++ gstPlugins
        );

        # winit loads these libraries dynamically; retain them in the runtime RPATH.
        runtimeDependencies = lib.optionals isLinux (graphicsDeps ++ windowingDeps);

        dontUseSwiftpmBuild = true;
        dontUseSwiftpmCheck = true;
        dontUseSwiftpmInstall = true;
        doCheck = false;

        preFixup = lib.optionalString isLinux ''
            gappsWrapperArgs+=(
              --prefix PATH : "${lib.makeBinPath runtimeTools}"
              --prefix GST_PLUGIN_SYSTEM_PATH_1_0 : "${
                lib.makeSearchPathOutput "lib" "lib/gstreamer-1.0" gstPlugins
            }"
            )
        '';

        postInstall =
            ''
                docs="$out/share/doc/serein"
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
            ''
            + lib.optionalString isLinux ''
                install -Dm444 ${finalAttrs.src}/packaging/linux/serein.desktop \
                  $out/share/applications/org.serein.desktop.desktop
                substituteInPlace $out/share/applications/org.serein.desktop.desktop \
                  --replace-fail "Exec=serein" "Exec=$out/bin/serein"

                for icon in ${finalAttrs.src}/packaging/linux/hicolor/*/apps/*; do
                  [ -f "$icon" ] || continue
                  size=$(basename "$(dirname "$(dirname "$icon")")")
                  install -Dm444 "$icon" \
                    "$out/share/icons/hicolor/$size/apps/$(basename "$icon")"
                done
            ''
            + lib.optionalString isDarwin ''
                app="$out/Applications/Serein.app/Contents"
                install -Dm444 ${finalAttrs.src}/packaging/macos/Info.plist "$app/Info.plist"
                install -Dm444 ${finalAttrs.src}/packaging/macos/Serein.icns "$app/Resources/Serein.icns"
                mkdir -p "$app/MacOS"
                ln -s "$out/bin/serein" "$app/MacOS/serein"
                ln -s "$docs" "$app/Resources/documentation"
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
