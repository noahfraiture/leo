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
      system = "aarch64-darwin";
      pkgs = import nixpkgs {
        inherit system;
        config.allowUnfree = true;
        overlays = [
          # Overlay which add rust-bin to pkgs
          inputs.rust-overlay.overlays.default
          (final: prev: {
            rustToolchain = prev.rust-bin.stable.latest.default.override {
              extensions = [ "rust-src" ];
              targets = [ "wasm32-unknown-unknown" ];
            };
          })
        ];
      };
    in
    {
      devShells.${system}.default = pkgs.mkShell {

        packages = with pkgs; [
          just
          tailwindcss_4
          dioxus-cli
          bacon
          rustToolchain
        ];

        DIRENV = "Leo";
      };
    };
}
