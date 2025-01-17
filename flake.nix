# TODO: windows cross compilation not working
# asio-sys will not build because of some missing intrinsics
# refer to win-build script for a "works on my machine" configuration
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    naersk = {
      url = "github:nix-community/naersk";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      fenix,
      flake-utils,
      naersk,
      nixpkgs,
    }:

    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = (import nixpkgs) {
          inherit system;
        };

        buildTargets = {
          aarch64-darwin = {
            rustTarget = "aarch64-apple-darwin";
            crossSystemConfig = "aarch64-apple-darwin";
          };
          x86_64-darwin = {
            rustTarget = "x86_64-apple-darwin";
            crossSystemConfig = "x86_64-apple-darwin";
          };
          aarch64-linux = {
            rustTarget = "aarch64-unknown-linux-gnu";
            crossSystemConfig = "aarch64-unknown-linux-gnu";
          };
          x86_64-linux = {
            rustTarget = "x86_64-unknown-linux-gnu";
            crossSystemConfig = "x86_64-unknown-linux-gnu";
          };
          x86_64-windows = {
            crossSystemConfig = "x86_64-w64-mingw32";
            rustTarget = "x86_64-pc-windows-gnu";
          };
          universal-darwin = { };
        };

        makeSystemsFromNames =
          systemNames: callback:
          builtins.foldl' (acc: systemName: acc // { ${systemName} = callback systemName; }) { } systemNames;

        mkCrossPkgs =
          system: targetSystem:
          let
            crossSystem = buildTargets.${targetSystem}.crossSystemConfig;
          in
          import nixpkgs ({
            inherit system crossSystem;
            # crossOverlays = import ./nix/overlays.nix;
          });

        makeToolchain =
          system:
          with fenix.packages.${system};
          combine [
            minimal.rustc
            minimal.cargo
            targets.${buildTargets.${system}}.latest.rust-std
          ];
      in
      rec {
        devShells.default = pkgs.mkShell {
          # buildInputs = [
          #   (makeToolchain system buildTargets.${system}.rustTarget)
          # ];
        };

        packages = makeSystemsFromNames (builtins.attrNames buildTargets) (
          systemName:
          let
            toolchain = makeToolchain systemName;

            naersk' = naersk.lib.${system}.override {
              cargo = toolchain;
              rustc = toolchain;
            };
          in

          if systemName == "x86_64-pc-windows-gnu" then
            let
              asioSdk = pkgs.fetchzip {
                url = "https://download.steinberg.net/sdk_downloads/asiosdk_2.3.3_2019-06-14.zip";
                sha256 = "sha256-4x3OiaJvC1P6cozsjL1orDr3nTdgDQrh2hlU2hDDu2Q=";
              };
            in
            naersk'.buildPackage {
              src = ./.;
              strictDeps = true;

              depsBuildBuild = with pkgs; [
                pkgsCross.mingwW64.stdenv.cc
                pkgsCross.mingwW64.windows.pthreads
                # pkgs.libclang.dev
                # pkgs.libclang.lib
              ];

              CARGO_BUILD_TARGET = "${systemName}";
              CPAL_ASIO_DIR = "${asioSdk}";
              # CPLUS_INCLUDE_PATH = "${mingwIncludePath}:${llvmIncludePath}";

              # CC_x86_64_pc_windows_gnu = "x86_64-w64-mingw32-gcc";
              # CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS = "-L${pkgs.pkgsCross.mingwW64.windows.pthreads}/lib";
              # CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = "x86_64-w64-mingw32-gcc";
              # BINDGEN_EXTRA_CLANG_ARGS="-I${pkgs.libclang.lib}/lib/clang/16/include";

            }

          else if systemName == "universal-darwin" then
            let
              projectName = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.name;

              universalBinary = pkgs.stdenv.mkDerivation {
                name = "universal-darwin-binary";
                src = ./.;

                buildInputs = [ pkgs.libllvm ];

                buildCommand = ''
                  mkdir -p $out/bin
                  ${pkgs.libllvm}/bin/llvm-lipo -create \
                    ${packages.aarch64-darwin}/bin/${projectName} \
                    ${packages.x86_64-darwin}/bin/${projectName} \
                    -output $out/bin/${projectName}
                '';
              };
            in
            universalBinary

          else
            naersk'.buildPackage {
              src = ./.;
              strictDeps = true;
              CARGO_BUILD_TARGET = buildTargets.${systemName};
            }
        );

        defaultPackage = packages.${system};
      }
    );
}
