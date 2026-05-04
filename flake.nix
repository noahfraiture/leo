{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
  };

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-darwin"
      ];
      forSystems =
        f:
        nixpkgs.lib.genAttrs systems (
          system:
          f {
            pkgs = import nixpkgs {
              inherit system;
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

            buildInputs = [
            ];

            packages = with pkgs; [
            ];

            DIRENV = "dioxus";
          };
        }
      );
    };
}
