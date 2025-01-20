# TODO:
# - windows cross compilation not working
#   asio-sys will not build because of some missing intrinsics
#   refer to script/build-windows-cross for a "works on my machine" configuration
# - get cross complied glibc version patch working
#
# refs:
# https://nixos.wiki/wiki/C#Override_binutils
# https://nixos.wiki/wiki/Cross_Compiling
# https://github.com/NixOS/nixpkgs/issues/129595
# https://crane.dev/examples/cross-rust-overlay.html
# https://github.com/haskell/clc-stackage/pull/10/files
# https://github.com/NixOS/nixpkgs/pull/144747
# https://discourse.nixos.org/t/how-do-i-pin-a-specific-version-of-glibc-using-shell-nix/11755/4
# https://discourse.nixos.org/t/how-to-pin-the-glibc-version-to-2-19/55656/4

{
  # imports
  system,
  systemsFrom,
  buildTargets,
  mkPkgs,
  mkToolchain,
  naersk,

  # pkgs
  pkg-config,
  fetchzip,
  lib,
  stdenv,
  llvm,
# mingwW64,
}:
with builtins;
let
  prefixCross =
    attrs:
    mapAttrs (name: value: value) (
      listToAttrs (
        map (name: {
          name = "cross-" + name;
          value = attrs.${name};
        }) (attrNames attrs)
      )
    );

  crossTargets =
    with lib;
    (filterAttrs (name: _: name != system) buildTargets)
    // {
      x86_64-windows = {
        crossSystemConfig = "x86_64-w64-mingw32";
        rustTarget = "x86_64-pc-windows-gnu";
      };
    };
in
prefixCross (
  systemsFrom (attrNames crossTargets) (
    crossSystem:
    let
      rustTarget = crossTargets.${crossSystem}.rustTarget;
      crossSystemConfig = crossTargets.${crossSystem}.crossSystemConfig;
      pkgsCross = mkPkgs { inherit system crossSystemConfig; };
      rsCrossToolchain = mkToolchain { inherit system rustTarget; };
      naersk' = naersk.lib.${system}.override {
        cargo = rsCrossToolchain;
        rustc = rsCrossToolchain;
      };
    in

    # this does not work at all lol
    if crossSystem == "x86_64-windows" then
      let
        asioSdk = fetchzip {
          url = "https://download.steinberg.net/sdk_downloads/asiosdk_2.3.3_2019-06-14.zip";
          sha256 = "sha256-4x3OiaJvC1P6cozsjL1orDr3nTdgDQrh2hlU2hDDu2Q=";
        };
      in
      naersk'.buildPackage rec {
        src = ../.;
        strictDeps = true;

        depsBuildBuild = [
          pkgsCross.stdenv.cc
          pkgsCross.windows.pthreads
        ];

        CPAL_ASIO_DIR = asioSdk;
        CARGO_BUILD_TARGET = rustTarget;

        CC = "${pkgsCross.stdenv.cc}/bin/${pkgsCross.stdenv.cc.targetPrefix}cc";
        TARGET_CC = "${pkgsCross.stdenv.cc}/bin/${pkgsCross.stdenv.cc.targetPrefix}cc";
        TARGET_AR = "${pkgsCross.stdenv.cc}/bin/${pkgsCross.stdenv.cc.targetPrefix}ar";

        CPLUS_INCLUDE_PATH = "$CPLUS_INCLUDE_PATH:${pkgsCross.stdenv.cc.libc.dev}/include:${llvm.dev}/include";

        CARGO_BUILD_RUSTFLAGS = [
          "-C"
          "linker=${CC}"
        ];

        # cp -p "/opt/homebrew/opt/mingw-w64/toolchain-x86_64/x86_64-w64-mingw32/lib/libstdc++-6.dll" $X86_BUILD_DIR
        # "${pkgsCross.stdenv.cc.lib}/lib/libstdc++-6.dll"
        # cp -p "/opt/homebrew/opt/mingw-w64/toolchain-x86_64/x86_64-w64-mingw32/lib/libgcc_s_seh-1.dll" $X86_BUILD_DIR
        # "${pkgsCross.stdenv.cc.lib}/lib/libgcc_s_seh-1.dll"
        # cp -p "/opt/homebrew/opt/mingw-w64/toolchain-x86_64/x86_64-w64-mingw32/bin/libwinpthread-1.dll" $X86_BUILD_DIR
        # ???
      }

    else
      naersk'.buildPackage (rec {
        src = ../.;
        strictDeps = true;
        CARGO_BUILD_TARGET = rustTarget;

        nativeBuildInputs = [ ] ++ lib.optionals stdenv.hostPlatform.isLinux [ pkg-config ];

        buildInputs =
          [ ]
          ++ lib.optionals stdenv.hostPlatform.isLinux [
            pkgsCross.alsa-lib.dev
            pkgsCross.fontconfig.dev
          ];

        CC = "${pkgsCross.stdenv.cc}/bin/${pkgsCross.stdenv.cc.targetPrefix}cc";
        TARGET_CC = CC;
        TARGET_AR = "${pkgsCross.stdenv.cc}/bin/${pkgsCross.stdenv.cc.targetPrefix}ar";

        CARGO_BUILD_RUSTFLAGS = [
          "-C"
          "linker=${CC}"
        ];
      })
  )
)
