{
  description = "ESP dev shell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    esp32 = {
      url = "github:leighleighleigh/esp-rs-nix";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-utils.follows = "flake-utils";
    };
  };

  outputs = {
    nixpkgs,
    flake-utils,
    esp32,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = import nixpkgs {
          inherit system;
        };
        libs-clang = pkgs.fetchzip {
          url = "https://github.com/espressif/llvm-project/releases/download/esp-19.1.2_20250225/libs-clang-esp-19.1.2_20250225-x86_64-linux-gnu.tar.xz";
          hash = "sha256-TUQ2UTyEVjYbcRw2OK1NFRYezJPViu50eG//A2vzwh8=";
        };
      in {
        devShells.default = pkgs.mkShell {
          LIBCLANG_PATH = "${libs-clang}/lib";

          inputsFrom = [
            esp32.devShells.${system}.default
          ];

          shellHook = ''
            export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath [pkgs.zlib]}:$LD_LIBRARY_PATH"
            export LD_LIBRARY_PATH="${pkgs.stdenv.cc.cc.lib.outPath}/lib:$LD_LIBRARY_PATH"
          '';

          packages = with pkgs; [
            clang
            esp-generate
            cargo-feature
          ];
        };
      }
    );
}
