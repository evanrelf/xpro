{
  inputs = {
    crane.url = "github:ipetkov/crane";
    fenix = {
      url = "github:nix-community/fenix/monthly";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-parts = {
      url = "github:hercules-ci/flake-parts";
      inputs.nixpkgs-lib.follows = "nixpkgs";
    };
    jj-hunk = {
      url = "github:laulauland/jj-hunk/v0.5.1";
      flake = false;
    };
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
    systems.url = "github:nix-systems/default";
  };

  outputs = inputs:
    inputs.flake-parts.lib.mkFlake { inherit inputs; } {
      systems = import inputs.systems;

      perSystem = { config, inputs', lib, pkgs, system, ... }:
        let
          toolchain = inputs'.fenix.packages.combine [
            inputs'.fenix.packages.default.toolchain
            inputs'.fenix.packages.complete.rust-src
          ];

          crane =
            (inputs.crane.mkLib pkgs).overrideToolchain toolchain;

          commonArgs = {
            pname = "xpro";
            version = "0.0.0";
            src = crane.cleanCargoSource ./.;
            strictDeps = true;
          };

          cargoArtifacts = crane.buildDepsOnly commonArgs;

          jj-hunk = pkgs.rustPlatform.buildRustPackage {
            pname = "jj-hunk";
            version = "0.5.1";
            src = inputs.jj-hunk;
            cargoHash = "sha256-tO4oGY92AieYb1SY3ylWSkOlcIKadbZLKOY6nTzXo48=";
            nativeCheckInputs = [ pkgs.jujutsu ];
          };
        in
        {
          packages = {
            default =
              crane.buildPackage (commonArgs // {
                inherit cargoArtifacts;
                cargoExtraArgs = "--bin xpro";
              });

            docs = config.checks.doc;
          };

          checks.clippy =
            crane.cargoClippy (commonArgs // { inherit cargoArtifacts; });

          checks.doc =
            crane.cargoDoc (commonArgs // {
              inherit cargoArtifacts;
              # Generate a root `index.html` listing all crates (nightly only)
              RUSTDOCFLAGS = "-Zunstable-options --enable-index-page";
            });

          checks.test =
            crane.cargoTest (commonArgs // { inherit cargoArtifacts; });

          devShells.default =
            crane.devShell {
              checks = config.checks;
              packages = with pkgs; [
                exiftool
                imagemagick
                jj-hunk
              ];
            };
        };
    };
}
