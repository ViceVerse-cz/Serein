{
    description = "Tiny, performant, native Discord client written in Rust (egui/wgpu)";

    inputs = {
        # Swift 6 is required by the macOS capture bridge; not yet in nixos-unstable.
        nixpkgs.url = "github:NixOS/nixpkgs/caa0ccbe110ef30193d17b033008c680ba60799b";
        flake-parts = {
            url = "github:hercules-ci/flake-parts";
            inputs.nixpkgs-lib.follows = "nixpkgs";
        };
    };

    outputs = inputs @ {
        self,
        nixpkgs,
        flake-parts,
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
            }: {
                packages.serein = pkgs.callPackage ./nix/package.nix {};
                packages.default = config.packages.serein;

                devShells.default = pkgs.mkShell {
                    name = "serein-dev";

                    inputsFrom = [config.packages.serein];
                };
            };
        };
}
