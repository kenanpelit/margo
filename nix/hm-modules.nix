self: {
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.wayland.windowManager.margo;
  selflib = import ./lib.nix lib;

  configText =
    lib.optionalString cfg.systemd.enable ''
      exec-once = ${pkgs.dbus}/bin/dbus-update-activation-environment --systemd ${lib.concatStringsSep " " cfg.systemd.variables}
    ''
    + lib.optionalString (cfg.settings != {}) (
      selflib.toMargo {
        topCommandsPrefixes = cfg.topPrefixes;
        bottomCommandsPrefixes = cfg.bottomPrefixes;
      }
      cfg.settings
    )
    + cfg.extraConfig;

  # The config file, rejected at build time when `mctl check-config` finds
  # an error in it (the same check `mctl reload` runs).
  configFile =
    pkgs.runCommand "margo-config.conf" {
      text = configText;
      passAsFile = ["text"];
    } ''
      cp "$textPath" "$out"
      ${lib.optionalString cfg.checkConfig ''
        HOME="$TMPDIR" XDG_CONFIG_HOME="$TMPDIR" \
          ${cfg.package}/bin/mctl check-config --config "$out"
      ''}
    '';
in {
  options.wayland.windowManager.margo = with lib; {
    enable = mkEnableOption "margo, a Rust/Smithay Wayland compositor with the mshell desktop shell";

    package = mkOption {
      type = types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.margo;
      defaultText = literalExpression "margo.packages.\${pkgs.stdenv.hostPlatform.system}.margo";
      description = ''
        The margo package to use. The login session, the PAM service the
        lock screen needs and the portal come from the NixOS module
        (`programs.margo`) or the distribution package; this module
        installs the package for the user and writes the compositor config.
      '';
    };

    settings = mkOption {
      type = with types; let
        valueType =
          nullOr (oneOf [
            bool
            int
            float
            str
            path
            (attrsOf valueType)
            (listOf valueType)
          ])
          // {
            description = "margo configuration value";
          };
      in
        attrsOf valueType;
      default = {};
      description = ''
        margo configuration (`~/.config/margo/config.conf`) written in Nix.
        Nested attributes are flattened with `_` (`animation.duration_open =
        400` becomes `animation_duration_open = 400`), lists become repeated
        keys (`bind`, `windowrule`, …), and the special `keymode` attribute
        holds one attribute set of bindings per key mode. See
        `config.example.conf` in the package's `share/doc/margo`.
      '';
      example = literalExpression ''
        {
          border_radius = 6;
          animations = 1;
          animation_duration_open = 400;
          bind = [
            "SUPER,r,reload_config"
            "SUPER,Return,spawn,foot"
            "ALT,R,setkeymode,resize"
          ];
          keymode.resize.bind = [
            "NONE,Left,resizewin,-10,0"
            "NONE,Escape,setkeymode,default"
          ];
        }
      '';
    };

    extraConfig = mkOption {
      type = types.lines;
      default = "";
      description = "Lines appended verbatim to `~/.config/margo/config.conf`.";
      example = ''
        source = ~/.config/margo/binds.conf
      '';
    };

    topPrefixes = mkOption {
      type = with types; listOf str;
      default = [];
      description = "Keys starting with one of these prefixes are written first.";
      example = ["source"];
    };

    bottomPrefixes = mkOption {
      type = with types; listOf str;
      default = [];
      description = "Keys starting with one of these prefixes are written last.";
      example = ["source"];
    };

    systemd = {
      enable = mkOption {
        type = types.bool;
        default = pkgs.stdenv.isLinux;
        example = false;
        description = ''
          Import {option}`systemd.variables` into the systemd and D-Bus user
          environment when margo starts (an `exec-once` line in the
          generated config), so user services and D-Bus-activated apps see
          the session. The uwsm session entry does this as well; this
          covers a margo started without uwsm.
        '';
      };
      variables = mkOption {
        type = with types; listOf str;
        default = [
          "DISPLAY"
          "WAYLAND_DISPLAY"
          "XDG_CURRENT_DESKTOP"
          "XDG_SESSION_TYPE"
          "NIXOS_OZONE_WL"
          "XCURSOR_THEME"
          "XCURSOR_SIZE"
        ];
        example = ["--all"];
        description = "Environment variables imported into the systemd and D-Bus user environment.";
      };
    };

    checkConfig = mkOption {
      type = types.bool;
      default = true;
      description = ''
        Validate the generated config with `mctl check-config` at build
        time. Turn it off when the config `source`s files that only exist
        in the home directory.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    home.packages = [cfg.package];
    xdg.configFile."margo/config.conf" = lib.mkIf (cfg.settings != {} || cfg.extraConfig != "" || cfg.systemd.enable) {
      source = configFile;
    };
  };
}
