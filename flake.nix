{
  description = "ollama_load_balancer nix flake";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  inputs.flake-utils.url = "github:numtide/flake-utils";

  outputs =
    inputs@{
      self,
      nixpkgs,
      flake-utils,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        ollama_load_balancer = let
          myArch = pkgs.stdenv.hostPlatform.uname.processor;
          # OS-specific deviations
          osAttrs = if pkgs.stdenv.isLinux then {
            nativeBuildInputs = with pkgs; [
              pkg-config
              openssl
            ];
            OPENSSL_LIB_DIR = "${pkgs.lib.getLib pkgs.openssl}/lib";
            OPENSSL_DIR = "${pkgs.lib.getDev pkgs.openssl}";
          } else {}; # nothing extra needed for MacOS to build yet
        in pkgs.rustPlatform.buildRustPackage (rec {
          pname = "ollama_load_balancer";
          version = "1.0.3";

          cargoLock.lockFile = ./Cargo.lock;
          src = pkgs.lib.cleanSource ./.;
        } // osAttrs);
      in
      {
        packages = rec {
          default = ollama_load_balancer;
        };

        formatter = pkgs.nixfmt-rfc-style;
      }
    );
}
