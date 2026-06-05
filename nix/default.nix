{
  lib,
  rustPlatform,
  # ── native (build-time tools) ──────────────────────────────────────────────
  pkg-config,
  wrapGAppsHook4, # GTK4 app: GSettings schemas, pixbuf loaders, icon themes
  wayland-scanner,
  git,
  # ── libraries (linked) ─────────────────────────────────────────────────────
  wayland,
  libxkbcommon,
  libinput,
  seatd, # libseat.so
  mesa, # DRM/KMS driver
  libgbm, # split out of mesa (≥ nixpkgs 24.11)
  libGL,
  pixman,
  libdrm,
  systemd, # libudev (smithay udev backend) + libsystemd
  libgudev,
  glib,
  pcre2, # regex crate PCRE backend → window-rule regexes
  pam, # mlock / mlogind PAM auth
  cairo, # mlock software renderer
  pango, # text shaping (mlock + mshell)
  dbus,
  libnotify,
  gtk4,
  gtk4-layer-shell,
  gdk-pixbuf,
  graphene,
  fontconfig,
  freetype,
  gst_all_1, # media widget: album art + sound previews
  alsa-lib,
  libpulseaudio,
  pipewire,
  mpv-unwrapped, # mplay links libmpv (cargo:rustc-link-lib=mpv)
  xwayland,
  # ── runtime PATH tools (spawned by the compositor / shell) ──────────────────
  uwsm,
  xdg-desktop-portal,
  xdg-desktop-portal-gtk,
  grim,
  slurp,
  wl-clipboard,
  enableXWayland ? true,
}:
rustPlatform.buildRustPackage {
  pname = "margo";
  version = "0.1.0";

  src = builtins.path {
    path = ../.;
    name = "margo-source";
  };

  cargoLock = {
    lockFile = ../Cargo.lock;
    # smithay is pinned to a git rev, so Nix needs its fetchgit NAR hash.
    # When bumping the smithay rev (Cargo.toml), recompute with:
    #   nix run nixpkgs#nix-prefetch-git -- --quiet \
    #     --url https://github.com/Smithay/smithay.git --rev <newrev>
    # and paste the `hash` field below.
    outputHashes = {
      "smithay-0.7.0" = "sha256-TV/GTfSvgfVwIFUGoASU7xm38opIBLjLMf1HeNTW07U=";
    };
  };

  # The workspace tests want a live Wayland/D-Bus/GPU seat; skip in the sandbox.
  doCheck = false;

  nativeBuildInputs = [
    pkg-config
    rustPlatform.bindgenHook # libclang for bindgen (pam, lutgen C-FFI)
    wrapGAppsHook4
    wayland-scanner
    git
  ];

  buildInputs =
    [
      # compositor / smithay
      wayland
      libxkbcommon
      libinput
      seatd
      mesa
      libgbm
      libGL
      pixman
      libdrm
      systemd
      libgudev
      pcre2
      # auth + text + ipc (mlock, mlogind, mshell)
      pam
      cairo
      pango
      glib
      dbus
      libnotify
      # mshell (gtk4 + relm4)
      gtk4
      gtk4-layer-shell
      gdk-pixbuf
      graphene
      fontconfig
      freetype
      # media: gstreamer + audio
      gst_all_1.gstreamer
      gst_all_1.gst-plugins-base
      gst_all_1.gst-plugins-good
      alsa-lib
      libpulseaudio
      pipewire
      # mplay → libmpv
      mpv-unwrapped
    ]
    ++ lib.optionals enableXWayland [xwayland];

  # Tools the compositor/shell spawn at runtime go on PATH; wrapGAppsHook4
  # threads gappsWrapperArgs into every wrapped binary.
  preFixup = ''
    gappsWrapperArgs+=(
      --prefix PATH : ${lib.makeBinPath ([
      uwsm
      xdg-desktop-portal
      xdg-desktop-portal-gtk
      grim
      slurp
      wl-clipboard
      mpv-unwrapped
    ]
    ++ lib.optionals enableXWayland [xwayland])}
    )
  '';

  passthru = {
    providedSessions = ["margo"];
  };

  meta = {
    mainProgram = "margo";
    description = "Feature-rich Wayland compositor + desktop shell (Rust/Smithay rewrite of mango/dwl)";
    homepage = "https://github.com/kenanpelit/margo";
    license = lib.licenses.gpl3Plus;
    maintainers = [];
    platforms = lib.platforms.linux;
  };
}
