{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };

  };

  outputs =
    { nixpkgs, ... }@inputs:
    let
      systems = [ "aarch64-darwin" ];
      forSystems =
        f:
        nixpkgs.lib.genAttrs systems (
          system:
          f {
            pkgs = import nixpkgs {
              inherit system;
              overlays = [
                # Overlay which add rust-bin to pkgs
                inputs.rust-overlay.overlays.default
                # Custom overlay to augment rust-bin with extension rust-src
                (final: prev: {
                  rustToolchain = prev.rust-bin.stable.latest.default.override {
                    extensions = [ "rust-src" ];
                  };
                })
              ];
              config.allowUnfree = true;
            };
          }
        );
    in
    {
      devShells = forSystems (
        { pkgs }:
        {
          default = pkgs.mkShell {

            packages = with pkgs; [
              go-task
              cargo
              cargo-watch
              ffmpeg
              rustToolchain
              nodejs
              pnpm_10
            ];

            DIRENV = "video-analysis";

            shellHook = ''
              if [ -f .local ]; then
                set -a
                . ./.local
                set +a
              fi
            '';
          };
        }
      );
    };
}
