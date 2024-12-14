# THIS FLAKE IS INCOMPLETE AND WILL NOT BUILD
# asio-sys will not build because of some missing intrinsics 
# refer to win-build script for a "works on my machine" configuration
 
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
        };

        mingwIncludePath ="/opt/homebrew/cellar/mingw-w64/12.0.0_1/toolchain-x86_64/x86_64-w64-mingw32/include";
        llvmIncludePath = "/opt/homebrew/opt/llvm/include";

        # mingwIncludePath ="${pkgs.pkgsCross.mingwW64.windows.mingw_w64_headers}/include";
        # llvmIncludePath = "${pkgs.llvmPackages_19.libclang.lib}/lib/clang/19/include";
        # llvmIncludePath = "${pkgs.llvmPackages_17.clang-unwrapped.dev}/include"; ------ has clang/Basic/BuiltinsX86_64.def
        # llvmIncludePath = "${pkgs.libclang.lib}/lib/clang/16/include";
        # llvmIncludePath = "${pkgs.libclang.dev}/include";

      in rec {
        defaultPackage = packages.x86_64-pc-windows-gnu;

        packages.x86_64-pc-windows-gnu = naersk'.buildPackage {
          src = ./.;
          strictDeps = true;

          depsBuildBuild = with pkgs; [
            pkgsCross.mingwW64.stdenv.cc
            pkgsCross.mingwW64.windows.pthreads
            # pkgs.libclang.dev
            # pkgs.libclang.lib
          ];

          CARGO_BUILD_TARGET = "x86_64-pc-windows-gnu";
          CPAL_ASIO_DIR = "${asioSdk}";
          CPLUS_INCLUDE_PATH = "${mingwIncludePath}:${llvmIncludePath}";

          # CC_x86_64_pc_windows_gnu = "x86_64-w64-mingw32-gcc";
          # CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS = "-L${pkgs.pkgsCross.mingwW64.windows.pthreads}/lib";
          # CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = "x86_64-w64-mingw32-gcc";
          # BINDGEN_EXTRA_CLANG_ARGS="-I${pkgs.libclang.lib}/lib/clang/16/include";

        };
      }
    );
}
