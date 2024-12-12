{
  inputs = {
    fenix.url = "github:nix-community/fenix";
    flake-utils.url = "github:numtide/flake-utils";
    naersk.url = "github:nix-community/naersk";
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  };

  outputs = { self, fenix, flake-utils, naersk, nixpkgs }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = (import nixpkgs) {
          inherit system;
        };

        asioSdk = pkgs.fetchzip {
          url = "https://download.steinberg.net/sdk_downloads/asiosdk_2.3.3_2019-06-14.zip";
          sha256 = "sha256-4x3OiaJvC1P6cozsjL1orDr3nTdgDQrh2hlU2hDDu2Q=";
        };

        toolchain = with fenix.packages.${system};
          combine [
            minimal.rustc
            minimal.cargo
            targets.x86_64-pc-windows-gnu.latest.rust-std
          ];

        naersk' = naersk.lib.${system}.override {
          cargo = toolchain;
          rustc = toolchain;
          stdenv = pkgs.llvmPackages.stdenv;
        };

        clangDev = /nix/store/lqhmclhp5ibg1gwl0ly216vlvv9d43xl-clang-18.1.8-dev;
        mingwIncludePath ="${pkgs.pkgsCross.mingwW64.windows.mingw_w64_headers}/include";
        # llvmIncludePath = "${pkgs.llvmPackages_19.libllvm.dev}/include";
        llvmIncludePath = "${clangDev}/include";


      in rec {
        defaultPackage = packages.x86_64-pc-windows-gnu;

        packages.x86_64-pc-windows-gnu = naersk'.buildPackage {
          src = ./.;
          strictDeps = true;

          CARGO_BUILD_TARGET = "x86_64-pc-windows-gnu";
          CPAL_ASIO_DIR = "${asioSdk}";
          CPLUS_INCLUDE_PATH =  "${mingwIncludePath}:${llvmIncludePath}";

          depsBuildBuild = with pkgs; [
            pkgsCross.mingwW64.stdenv.cc
            pkgsCross.mingwW64.windows.pthreads
          ];

        };
      }
    );
}
