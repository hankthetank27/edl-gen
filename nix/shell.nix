{
  # imports
  rsToolchain,

  # pkgs
  mkShell,
  pkg-config,
  lib,
  stdenv,
  fontconfig,
  alsa-lib,
}:
mkShell {
  packages = [ ] ++ lib.optionals stdenv.hostPlatform.isLinux [ pkg-config ];
  buildInputs =
    [
      rsToolchain
    ]
    ++ lib.optionals stdenv.hostPlatform.isLinux [
      alsa-lib
      fontconfig
    ];
}
