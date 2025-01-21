{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
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
      naersk,
      nixpkgs,
    }:

    with builtins;
    let
      projectName = (fromTOML (readFile ./Cargo.toml)).package.name;

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
      };

      systemsFrom = systems: f: foldl' (acc: (system: acc // { ${system} = f system; })) { } systems;

      mkPkgs =
        {
          system,
          crossSystemConfig ? null,
        }:
        (import nixpkgs) (
          {
            inherit system;
          }
          // (
            if crossSystemConfig == null then
              { }
            else
              {
                crossSystem.config = crossSystemConfig;
              }
          )
        );

      mkToolchain =
        {
          system,
          rustTarget ? buildTargets.${system}.rustTarget,
        }:
        with fenix.packages.${system};
        combine [
          stable.rustc
          stable.cargo
          targets.${rustTarget}.stable.rust-std
        ];
    in
    {
      devShells.default = systemsFrom (attrNames buildTargets) (
        system:
        let
          pkgs = mkPkgs { inherit system; };
          rsToolchain = mkToolchain { inherit system; };
        in
        with pkgs;
        {
          defualt = callPackage ./nix/shell.nix { inherit rsToolchain; };
        }
      );

      packages = systemsFrom (attrNames buildTargets) (
        system:
        let
          pkgs = mkPkgs { inherit system; };
          rsToolchain = mkToolchain { inherit system; };
        in
        with pkgs;
        {

          default = callPackage ./nix/build.nix {
            inherit
              buildTargets
              system
              rsToolchain
              naersk
              ;
          };

          universal-darwin = stdenv.mkDerivation {
            name = "MacOS Universal Binary";
            src = ./.;
            buildInputs = [ libllvm ];
            buildCommand = ''
              mkdir -p $out/bin
              ${libllvm}/bin/llvm-lipo -create \
                ${self.packages.aarch64-darwin.default}/bin/${projectName} \
                ${self.packages.x86_64-darwin.default}/bin/${projectName} \
                -output $out/bin/${projectName}
            '';
          };
        }

        // callPackage ./nix/build-cross.nix {
          inherit
            system
            systemsFrom
            buildTargets
            mkPkgs
            mkToolchain
            naersk
            ;
        }
      );
    };
}
