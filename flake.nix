{
  description = "drillgame development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    cargoMacheteSrc = {
      url = "github:BSteffaniak/cargo-machete/ignored-dirs";
      flake = false;
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      rust-overlay,
      cargoMacheteSrc,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };
        cargoMachete = pkgs.rustPlatform.buildRustPackage {
          pname = "cargo-machete";
          version = "ignored-dirs";
          src = cargoMacheteSrc;
          cargoLock = {
            lockFile = "${cargoMacheteSrc}/Cargo.lock";
          };
          doCheck = false;
        };
        rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
        linuxGraphicsLibraries = pkgs.lib.optionals pkgs.stdenv.isLinux (
          with pkgs; [
            libGL
            wayland
            xorg.libX11
            xorg.libXcursor
            xorg.libXi
            xorg.libXinerama
            xorg.libXrandr
          ]
        );
      in
      {
        devShells.default = pkgs.mkShell {
          buildInputs = [
            rustToolchain
            cargoMachete
            pkgs.cargo-deny
            pkgs.cmake
            pkgs.pkg-config
            pkgs.clang
            pkgs.llvmPackages.libclang
            pkgs.fish
          ] ++ linuxGraphicsLibraries;

          LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
          RUST_BACKTRACE = "1";

          shellHook = ''
            echo "drillgame development environment loaded"
            echo "Available tools:"
            echo "  - cargo ($(cargo --version))"
            echo "  - rustc ($(rustc --version))"
            echo "  - clippy ($(cargo clippy --version))"
            echo "  - cargo-deny ($(cargo deny --version))"
            echo "  - cargo-machete ($(cargo machete --version))"
            echo "  - cmake ($(cmake --version | head -1))"

            # Only exec fish if we're in an interactive shell (not running a command)
            if [ -z "$IN_NIX_SHELL_FISH" ] && [ -z "$BASH_EXECUTION_STRING" ]; then
              case "$-" in
                *i*) export IN_NIX_SHELL_FISH=1; exec fish ;;
              esac
            fi
          '';
        };
      }
    );
}
