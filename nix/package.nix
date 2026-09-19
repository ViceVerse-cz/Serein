{
    lib,
    stdenv,
    rustPlatform,
    pkg-config,
    cmake,
    makeWrapper,
    swift,
    swiftpm,
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
    libglvnd,
    alsa-lib,
    gst_all_1,
    libX11,
    libXi,
    libXrandr,
    libXcursor,
    bubblewrap,
    xdg-dbus-proxy,
}:
rustPlatform.buildRustPackage rec {
    pname = "serein";
    version = "1.0.0-nightly.20260914.14";

    src = ../.;

    cargoLock = {
        lockFile = "${src}/Cargo.lock";
        outputHashes = {
            "ecolor-0.36.2" = "sha256-G9x6P6ksfbEx9sTqpgzvbuMxjDUTgW2ghImLB6nUx94=";
        };
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
        ++ lib.optionals stdenv.hostPlatform.isLinux [
            wrapGAppsHook4
            autoPatchelfHook
        ]
        ++ lib.optionals stdenv.hostPlatform.isDarwin [
            swift
            swiftpm
        ];

    dontUseSwiftpmBuild = true;
    dontUseSwiftpmCheck = true;

    buildInputs =
        lib.optionals stdenv.hostPlatform.isLinux [
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
            wayland
            libxkbcommon
            libX11
            libXi
            libXrandr
            libXcursor
            fontconfig
            vulkan-loader
            libGL
            libGLX
            libglvnd
            alsa-lib
            gst_all_1.gstreamer
            gst_all_1.gst-plugins-bad
            gst_all_1.gst-plugins-base
            gst_all_1.gst-plugins-good
            gst_all_1.gst-libav
        ]
        ++ lib.optionals stdenv.hostPlatform.isDarwin [
            apple-sdk_15
        ];

    runtimeDependencies = lib.optionals stdenv.hostPlatform.isLinux [
        vulkan-loader
        libGL
        libGLX
        libglvnd
    ];

    doCheck = false;

    preFixup = lib.optionalString stdenv.hostPlatform.isLinux ''
        gappsWrapperArgs+=(
          --prefix PATH : "${
            lib.makeBinPath [
                bubblewrap
                xdg-dbus-proxy
            ]
        }"
          --prefix GST_PLUGIN_SYSTEM_PATH : "${
            lib.makeSearchPathOutput "lib" "lib/gstreamer-1.0" [
                gst_all_1.gst-plugins-bad
                gst_all_1.gst-plugins-base
                gst_all_1.gst-plugins-good
                gst_all_1.gst-libav
            ]
        }"
        )
    '';

    postInstall =
        lib.optionalString stdenv.hostPlatform.isLinux ''
            install -Dm444 ${src}/packaging/linux/serein.desktop \
              $out/share/applications/org.serein.desktop.desktop
            substituteInPlace $out/share/applications/org.serein.desktop.desktop \
              --replace-fail "Exec=serein" "Exec=$out/bin/serein"

            for theme_dir in ${src}/packaging/linux/hicolor/*; do
              size=$(basename "$theme_dir")
              for icon in "$theme_dir"/apps/*; do
                if [ -f "$icon" ]; then
                  install -Dm444 "$icon" "$out/share/icons/hicolor/$size/apps/$(basename "$icon")"
                fi
              done
            done
        ''
        + lib.optionalString stdenv.hostPlatform.isDarwin ''
            mkdir -p "$out/Applications/Serein.app/Contents/MacOS" "$out/Applications/Serein.app/Contents/Resources"
            install -Dm444 ${src}/packaging/macos/Info.plist "$out/Applications/Serein.app/Contents/Info.plist"
            install -Dm444 ${src}/packaging/macos/Serein.icns "$out/Applications/Serein.app/Contents/Resources/Serein.icns"
            ln -s "$out/bin/serein" "$out/Applications/Serein.app/Contents/MacOS/serein"
        '';

    meta = with lib; {
        description = "Tiny, performant, native Discord client written in Rust (egui/wgpu)";
        homepage = "https://github.com/ViceVerse-cz/Serein";
        license = with licenses; [
            mit
            asl20
        ];
        platforms = platforms.linux ++ platforms.darwin;
        mainProgram = "serein";
    };
}
