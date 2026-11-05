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
        nixosModules.margo = import ./nix/nixos-modules.nix self;
        nixosModules.default = self.nixosModules.margo;
        homeManagerModules.margo = import ./nix/hm-modules.nix self;
        homeManagerModules.default = self.homeManagerModules.margo;
        # The name this flake used to export; home-manager now looks for
        # `homeManagerModules`.
        hmModules.margo = self.homeManagerModules.margo;
      };

      perSystem = {
        config,
        pkgs,
        ...
      }: let
        margo = pkgs.callPackage ./nix {};
        shellOverride = old: {
          nativeBuildInputs = old.nativeBuildInputs ++ [pkgs.rust-analyzer pkgs.clippy];
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
        # Evaluation-only check of the NixOS module: a minimal system with
        # `programs.margo.enable` must evaluate (option names, the session
        # package, the portal and unit wiring). Nothing is built.
        checks.nixos-module = let
          system = inputs.nixpkgs.lib.nixosSystem {
            inherit (pkgs.stdenv.hostPlatform) system;
            modules = [
              self.nixosModules.margo
              {
                programs.margo.enable = true;
                boot.loader.grub.enable = false;
                fileSystems."/" = {
                  device = "nodev";
                  fsType = "tmpfs";
                };
                system.stateVersion = "25.11";
              }
            ];
          };
          drv = builtins.unsafeDiscardStringContext system.config.system.build.toplevel.drvPath;
        in
          pkgs.runCommand "margo-nixos-module-eval" {} ''
            echo ${drv} > "$out"
          '';
        formatter = pkgs.alejandra;
      };
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
    };
}
