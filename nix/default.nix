{
  lib,
  rustPlatform,
  # ── native (build-time tools) ──────────────────────────────────────────────
  pkg-config,
  wrapGAppsHook4, # GTK4 apps: GSettings schemas, pixbuf loaders, icon themes
  wayland-scanner,
  glib, # glib-compile-resources for mtune's build.rs (+ the linked lib below)
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
  pcre2, # regex crate PCRE backend → window-rule regexes
  pam, # mlock / mlogind PAM auth
  cairo, # mlock software renderer
  pango, # text shaping (mlock + mshell)
  dbus,
  libnotify,
  gtk4,
  gtk4-layer-shell,
  libadwaita, # mtune window chrome
  gdk-pixbuf,
  graphene,
  fontconfig,
  freetype,
  gst_all_1, # media widget (album art, sound previews) + mtune's engine
  alsa-lib,
  libpulseaudio,
  pipewire,
  mpv-unwrapped, # mshellctl links libmpv (wallpaper engine)
  xwayland,
  # ── runtime PATH tools (spawned by the compositor / shell) ──────────────────
  uwsm,
  xdg-desktop-portal,
  xdg-desktop-portal-gtk,
  grim,
  slurp,
  wl-clipboard,
  enableXWayland ? true,
}: let
  # One source of truth for the version: the workspace manifest.
  version = (lib.importTOML ../Cargo.toml).workspace.package.version;
in
  rustPlatform.buildRustPackage {
    pname = "margo";
    inherit version;

    src = lib.fileset.toSource {
      root = ../.;
      # The cargo workspace plus the data files the install phase ships;
      # CI config, docs and the Nix files themselves stay out of the source
      # hash, so editing them doesn't rebuild the package.
      fileset = lib.fileset.difference ../. (lib.fileset.unions [
        (lib.fileset.maybeMissing ../.github)
        (lib.fileset.maybeMissing ../wiki)
        (lib.fileset.maybeMissing ../target)
        (lib.fileset.maybeMissing ../result)
        ../nix
        ../flake.nix
        ../flake.lock
      ]);
    };

    cargoLock = {
      lockFile = ../Cargo.lock;
      # smithay is a git dependency. Fetching it with builtins.fetchGit at
      # the rev Cargo.lock pins needs no hand-maintained output hash, so a
      # smithay bump can't leave the flake with a stale one.
      allowBuiltinFetchGit = true;
    };

    nativeBuildInputs = [
      pkg-config
      rustPlatform.bindgenHook # libclang for bindgen (pam, libspa, …)
      wrapGAppsHook4
      wayland-scanner
      glib
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
        # mshell (gtk4 + relm4), mtune (gtk4 + libadwaita)
        gtk4
        gtk4-layer-shell
        libadwaita
        gdk-pixbuf
        graphene
        fontconfig
        freetype
        # media: gstreamer (mtune links gstreamer-play from -bad) + audio
        gst_all_1.gstreamer
        gst_all_1.gst-plugins-base
        gst_all_1.gst-plugins-good
        gst_all_1.gst-plugins-bad
        alsa-lib
        libpulseaudio
        pipewire
        # mshellctl → libmpv
        mpv-unwrapped
      ]
      ++ lib.optionals enableXWayland [xwayland];

    # Three cargo invocations, exactly the PKGBUILD's build() groups — do NOT
    # merge them into one (which is what buildRustPackage's default build
    # does on a virtual workspace). Cargo unifies features per invocation:
    # building the shell crates together with the compositor turns on
    # zbus's `tokio` feature for the zbus margo links, and margo — which
    # drives zbus on async-io and never starts a Tokio runtime — panics at
    # startup ("there is no reactor running"). mtune gets its own
    # invocation for the same reason (async-io zbus, its own GStreamer
    # stack). The PKGBUILD's build() says which crate belongs where and why.
    buildPhase = ''
      runHook preBuild
      cargo build --frozen --release -j "$NIX_BUILD_CORES" \
        -p margo -p start-margo \
        -p mctl -p mlock -p mlayout -p mscreenshot -p mvisual -p mlogind -p mpower \
        -p mdots -p mcal -p mtm
      # mshell runs its root helper by absolute path, and the polkit policy
      # names the same one: point both at the store, not /usr/lib/mshell.
      export MSHELL_PRIVILEGED_HELPER="$out/lib/mshell/mshell-privileged"
      cargo build --frozen --release -j "$NIX_BUILD_CORES" \
        --features mshell/wasm-plugins \
        -p mshell -p mshellctl -p mshellshare -p mpicker -p mwizard \
        -p mkeys -p mvpn -p mgreet \
        -p margo-portal -p mshell-privileged
      cargo build --frozen --release -j "$NIX_BUILD_CORES" -p mtune
      runHook postBuild
    '';

    # The workspace tests want a live Wayland/D-Bus/GPU seat; CI runs them.
    doCheck = false;

    # Mirrors the PKGBUILD's package(): binaries, the uwsm session entry +
    # its wrapper scripts, the user units, the margo-portal backend, data
    # files and man pages — with every /usr path rewritten to $out.
    installPhase = ''
      runHook preInstall
      rel=target/release
      for bin in \
          margo start-margo \
          mctl mlock mlayout mscreenshot mvisual mlogind mgreet mpower \
          mshell mshellctl mshellshare mpicker mwizard mkeys mvpn mdots mcal mtm \
          mtune; do
        install -Dm755 "$rel/$bin" "$out/bin/$bin"
      done
      install -Dm755 "$rel/margo-portal" "$out/lib/margo/margo-portal"
      install -Dm755 "$rel/mshell-privileged" "$out/lib/mshell/mshell-privileged"

      # usr <file>: point the /usr (and /usr/local) paths at this package.
      usr() {
        substituteInPlace "$1" \
          --replace-quiet /usr/local/bin/ "$out/bin/" \
          --replace-quiet /usr/bin/ "$out/bin/" \
          --replace-quiet /usr/lib/margo/ "$out/lib/margo/" \
          --replace-quiet /usr/lib/mshell/ "$out/lib/mshell/"
      }

      # mshell's root helper's polkit actions (NixOS links share/polkit-1
      # of systemPackages); exec.path must be the store path pkexec runs.
      install -Dm644 mshell-crates/mshell-privileged/data/org.margo.mshell.privileged.policy \
        "$out/share/polkit-1/actions/org.margo.mshell.privileged.policy"
      usr "$out/share/polkit-1/actions/org.margo.mshell.privileged.policy"

      # Session: the uwsm-managed entry only (the PKGBUILD's package() says
      # why the bare `Exec=margo` entry is not offered).
      install -Dm755 contrib/sessions/margo-uwsm-session "$out/bin/margo-uwsm-session"
      install -Dm755 contrib/sessions/margo-session "$out/bin/margo-session"
      install -Dm644 contrib/sessions/margo-uwsm.desktop \
        "$out/share/wayland-sessions/margo-uwsm.desktop"
      usr "$out/share/wayland-sessions/margo-uwsm.desktop"
      install -Dm644 contrib/sessions/uwsm-env-margo "$out/etc/xdg/uwsm/env-margo"

      # User units (NixOS: systemd.packages; the module adds the wants).
      install -Dm644 assets/mshell.service "$out/lib/systemd/user/mshell.service"
      install -Dm644 mpower/mpower.service "$out/lib/systemd/user/mpower.service"
      install -Dm644 assets/margo-portal.service "$out/lib/systemd/user/margo-portal.service"
      for unit in "$out"/lib/systemd/user/*.service; do usr "$unit"; done

      # Portal backend: registration, D-Bus activation, preferences.
      install -Dm644 assets/margo.portal "$out/share/xdg-desktop-portal/portals/margo.portal"
      install -Dm644 assets/margo-portals.conf "$out/share/xdg-desktop-portal/margo-portals.conf"
      install -Dm644 assets/dbus/org.freedesktop.impl.portal.desktop.margo.service \
        "$out/share/dbus-1/services/org.freedesktop.impl.portal.desktop.margo.service"
      usr "$out/share/dbus-1/services/org.freedesktop.impl.portal.desktop.margo.service"

      # mtune: desktop entry, metainfo, D-Bus activation, GSettings schema
      # (compiled here; wrapGAppsHook4 puts it on XDG_DATA_DIRS), icons.
      install -Dm644 mtune/data/org.margo.Tune.desktop.in "$out/share/applications/org.margo.Tune.desktop"
      install -Dm644 mtune/data/org.margo.Tune.metainfo.xml "$out/share/metainfo/org.margo.Tune.metainfo.xml"
      install -Dm644 mtune/data/org.margo.Tune.service "$out/share/dbus-1/services/org.margo.Tune.service"
      usr "$out/share/dbus-1/services/org.margo.Tune.service"
      install -Dm644 mtune/data/org.margo.Tune.gschema.xml \
        "$out/share/gsettings-schemas/$name/glib-2.0/schemas/org.margo.Tune.gschema.xml"
      glib-compile-schemas "$out/share/gsettings-schemas/$name/glib-2.0/schemas"
      install -Dm644 mtune/data/icons/hicolor/scalable/apps/org.margo.Tune.svg \
        "$out/share/icons/hicolor/scalable/apps/org.margo.Tune.svg"
      install -Dm644 mtune/data/icons/hicolor/symbolic/apps/org.margo.Tune-symbolic.svg \
        "$out/share/icons/hicolor/symbolic/apps/org.margo.Tune-symbolic.svg"

      # Icons, the MargoMaterial icon theme, wallpapers, shell data.
      install -Dm644 docs/assets/margo-icon.svg "$out/share/icons/hicolor/scalable/apps/margo.svg"
      mkdir -p "$out/share/icons/MargoMaterial" "$out/share/mshell/scss"
      cp -a assets/icons/MargoMaterial/. "$out/share/icons/MargoMaterial/"
      install -Dm644 -t "$out/share/margo/wallpapers" assets/wallpapers/default.jpg assets/wallpapers/margo-hero.png
      install -Dm644 -t "$out/share/margo/mshell/profiles" mshell/examples/profiles/*.yaml
      install -Dm644 -t "$out/share/mshell/sounds" mshell-crates/mshell-sounds/assets/*.ogg
      cp -a mshell-crates/mshell-style/scss/. "$out/share/mshell/scss/"

      # Man pages, completions, examples, licenses.
      install -Dm644 -t "$out/share/man/man1" man/*.1
      for tool in mctl mshellctl mdots; do
        install -Dm644 "contrib/completions/$tool.bash" "$out/share/bash-completion/completions/$tool"
        install -Dm644 "contrib/completions/_$tool" "$out/share/zsh/site-functions/_$tool"
        install -Dm644 "contrib/completions/$tool.fish" "$out/share/fish/vendor_completions.d/$tool.fish"
      done
      install -Dm644 margo/src/config.example.conf "$out/share/doc/margo/config.example.conf"
      install -Dm644 -t "$out/share/doc/margo/layouts" mlayout/examples/layout_*.conf
      install -Dm644 -t "$out/share/licenses/margo" LICENSE licenses/*
      for lic in mlogind/LICENSE-MIT mlogind/LICENSE-APACHE; do
        install -Dm644 "$lic" "$out/share/licenses/margo/mlogind-$(basename "$lic")"
      done
      for lic in mtune/licenses/*; do
        install -Dm644 "$lic" "$out/share/licenses/margo/mtune-$(basename "$lic")"
      done
      runHook postInstall
    '';

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
          gtk4 # gtk4-launch: how mshell starts desktop apps
        ]
        ++ lib.optionals enableXWayland [xwayland])}
      )
    '';

    passthru = {
      # The wayland-sessions entry, for services.displayManager.sessionPackages.
      providedSessions = ["margo-uwsm"];
    };

    meta = {
      mainProgram = "margo";
      description = "Rust/Smithay Wayland tiling compositor with a first-party GTK4 desktop shell (mshell)";
      homepage = "https://github.com/kenanpelit/margo";
      license = lib.licenses.gpl3Plus;
      maintainers = [];
      platforms = lib.platforms.linux;
    };
  }
