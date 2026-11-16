{
    description = "Tiny, performant, native Discord client written in Rust (egui/wgpu)";

    inputs = {
        nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
        flake-parts = {
            url = "github:hercules-ci/flake-parts";
            inputs.nixpkgs-lib.follows = "nixpkgs";
        };
        rust-overlay = {
            url = "github:oxalica/rust-overlay/b70538330f20adc9665afa2976471eb5b85ebd1f";
            inputs.nixpkgs.follows = "nixpkgs";
        };
    };

    outputs = inputs @ {
        self,
        nixpkgs,
        flake-parts,
        rust-overlay,
        ...
    }:
        flake-parts.lib.mkFlake {inherit inputs;} {
            systems = [
                "x86_64-linux"
                "aarch64-darwin"
            ];

            perSystem = {
                config,
                pkgs,
                system,
                ...
            }: let
                rustBin = rust-overlay.lib.mkRustBin {} pkgs;
                rustToolchain = rustBin.fromRustupToolchainFile ./rust-toolchain.toml;
                buildRustToolchain = rustToolchain.override {extensions = [];};
                rustPlatform = pkgs.makeRustPlatform {
                    cargo = buildRustToolchain;
                    rustc = buildRustToolchain;
                };
            in {
                packages.serein = pkgs.callPackage ./nix/package.nix {inherit rustPlatform;};
                packages.default = config.packages.serein;

                devShells.default = pkgs.mkShell {
                    name = "serein-dev";

                    inputsFrom = [config.packages.serein];
                    packages = [rustToolchain];
                    FFMPEG_DIR = config.packages.serein.FFMPEG_DIR;
                };
            };
        };
}
