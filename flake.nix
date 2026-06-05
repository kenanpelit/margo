{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
  };

  outputs = {
    self,
    flake-parts,
    ...
  } @ inputs:
    flake-parts.lib.mkFlake {inherit inputs;} {
      imports = [
        inputs.flake-parts.flakeModules.easyOverlay
      ];

      flake = {
        hmModules.margo = import ./nix/hm-modules.nix self;
        nixosModules.margo = import ./nix/nixos-modules.nix self;
      };

      perSystem = {
        config,
        pkgs,
        ...
      }: let
        margo = pkgs.callPackage ./nix {};
        shellOverride = old: {
          nativeBuildInputs = old.nativeBuildInputs ++ [ pkgs.rust-analyzer pkgs.clippy ];
          buildInputs = old.buildInputs ++ [];
        };
      in {
        packages.default = margo;
        overlayAttrs = {
          inherit (config.packages) margo;
        };
        packages = {
          inherit margo;
        };
        devShells.default = margo.overrideAttrs shellOverride;
        formatter = pkgs.alejandra;
      };
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
    };
}
