{
  # imports
  system,
  buildTargets,
  rsToolchain,
  naersk,

  # pkgs
  lib,
  stdenv,
  pkg-config,
  alsa-lib,
  fontconfig,
}:

let
  rustTarget = buildTargets.${system}.rustTarget;
  naersk' = naersk.lib.${system}.override {
    cargo = rsToolchain;
    rustc = rsToolchain;
  };
in

naersk'.buildPackage (
  {
    src = ../.;
    strictDeps = true;

    nativeBuildInputs = [ ] ++ lib.optionals stdenv.hostPlatform.isLinux [ pkg-config ];

    buildInputs =
      [ ]
      ++ lib.optionals stdenv.hostPlatform.isLinux [
        alsa-lib.dev
        fontconfig.dev
      ];

    CARGO_BUILD_TARGET = rustTarget;
  }
  // lib.optionalAttrs stdenv.hostPlatform.isDarwin {
    MACOSX_DEPLOYMENT_TARGET = "10.7";
  }
)
