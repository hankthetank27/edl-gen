# TODO:
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
  lib,
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

  crossTargets = with lib; (filterAttrs (name: _: name != system) buildTargets);
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
      cargoToml = lib.importTOML ../Cargo.toml;
    in
    with pkgsCross;
    naersk'.buildPackage (rec {
      src = ../.;
      strictDeps = true;
      CARGO_BUILD_TARGET = rustTarget;
      version = cargoToml.workspace.package.version;

      nativeBuildInputs = [ ] ++ lib.optionals stdenv.hostPlatform.isLinux [ pkg-config ];

      buildInputs =
        [ ]
        ++ lib.optionals stdenv.hostPlatform.isLinux [
          alsa-lib.dev
          fontconfig.dev
        ];

      CC = "${stdenv.cc}/bin/${stdenv.cc.targetPrefix}cc";
      TARGET_CC = CC;
      TARGET_AR = "${stdenv.cc}/bin/${stdenv.cc.targetPrefix}ar";

      CARGO_BUILD_RUSTFLAGS = [
        "-C"
        "linker=${CC}"
      ];
    })
  )
)
