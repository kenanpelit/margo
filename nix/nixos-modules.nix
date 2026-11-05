self: {
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.programs.margo;
in {
  options.programs.margo = {
    enable = lib.mkEnableOption "margo, a Rust/Smithay Wayland compositor with the mshell desktop shell";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.margo;
      defaultText = lib.literalExpression "margo.packages.\${pkgs.stdenv.hostPlatform.system}.margo";
      description = "The margo package to use.";
    };

    shell.enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Start mshell (bar, menus, notifications, lock screen) with every
        margo session, as the Arch package does. The unit only runs in a
        margo session.
      '';
    };

    mpower.enable = lib.mkOption {
      type = lib.types.bool;
      default = config.services.power-profiles-daemon.enable;
      defaultText = lib.literalExpression "config.services.power-profiles-daemon.enable";
      description = ''
        Start mpower (automatic power profiles from CPU load and AC state)
        with every margo session. It drives power-profiles-daemon.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [cfg.package];
    # Wallpapers and starter profiles (share/margo) and the shell's data
    # (share/mshell) are looked up through XDG_DATA_DIRS.
    environment.pathsToLink = ["/share/margo" "/share/mshell"];

    # The login entry is the uwsm-managed session (share/wayland-sessions/
    # margo-uwsm.desktop); uwsm activates graphical-session.target, which
    # starts mshell. uwsm's own units come from its module.
    services.displayManager.sessionPackages = [cfg.package];
    programs.uwsm.enable = lib.mkDefault true;

    # mshell.service, mpower.service and margo-portal.service ship in the
    # package; NixOS ignores their [Install] sections, so the wants are
    # declared here.
    systemd.packages = [cfg.package];
    systemd.user.services.mshell.wantedBy = lib.mkIf cfg.shell.enable ["graphical-session.target"];
    systemd.user.services.mpower.wantedBy = lib.mkIf cfg.mpower.enable ["graphical-session.target"];

    # margo-portal serves ScreenCast / Screenshot / GlobalShortcuts; the gtk
    # backend serves the rest. The routing is the package's
    # margo-portals.conf (XDG_CURRENT_DESKTOP=margo).
    services.dbus.packages = [cfg.package];
    xdg.portal = {
      enable = lib.mkDefault true;
      extraPortals = [cfg.package pkgs.xdg-desktop-portal-gtk];
      configPackages = [cfg.package];
    };

    # The lockers (mlock, mshell's lock screen) authenticate against the
    # `mlock` PAM service.
    security.pam.services.mlock = {};
    security.polkit.enable = lib.mkDefault true;
    programs.xwayland.enable = lib.mkDefault true;
    services.graphical-desktop.enable = lib.mkDefault true;
  };
}
