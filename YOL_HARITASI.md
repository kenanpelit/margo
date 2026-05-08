# Margo Yol Haritası

> Güncellendi: 2026-05-08 (P1 sprint sonu)
> Branch: `main` (rust branch artık yok — tek dal)
> Durum: **P0 + P0+ polish + P1 protocol parity + P2 perf/akıcılık tamam.** Margo artık modern Wayland masaüstü protokollerinin tamamına paritede, on-demand redraw scheduler ile idle GPU sıfır, smithay'in primary/cursor/overlay plane scanout'ları açık, ve niri/mangowm-ext'in üstünde bir animasyon paketi (move/open/close/tag/focus/layer hepsi animeli, spring fizik motoru opt-in, snapshot-driven open animation ile ilk frame'de mid-curve). Sıradaki hedef P3: pencere yönetimi v2.

Bu dosya sadece vizyon listesi değil, yapılacak işlerin öncelik sırasıdır. Bir madde bitmeden "tamam" sayılması için alttaki kabul kriteri de sağlanmalı.

## Hızlı durum

| Blok | Madde | Durum |
|---|---|---|
| Çekirdek | UWSM, config, scroller, render, clipboard, layer, gamma, screencopy SHM, swipe gestures | ✅ |
| P0 | session_lock, idle_notifier, hotplug, debug log, move/resize, smoke test | ✅ (6/6) |
| P0+ polish | text_input + IM, multi-monitor lock cursor-tracking, force_unlock, focus oracle (niri pattern), layer-mutate detect, tagview, stack z-order, scroller jitter chain, niri-style resize transition + 2-texture crossfade, deferred initial map (CopyQ flicker), README + diag tracing, noctalia LockScreen multi-monitor dots | **✅ (12/12)** |
| **P1 protocol parity** | linux_dmabuf + drm-syncobj, dmabuf-screencopy, region-crop, blockout filter, pointer_constraints + relative_pointer, xdg_activation, output_management, presentation-time | **✅ (8/8)** |
| P2 | frame_clock, spring animasyon, open/close/tag/focus/layer anims, hw cursor, direct scanout, damage opt | **✅ (6/6)** |
| P3 | scratchpad, mango parity, CSD/SSD, noctalia IPC, XWayland HiDPI, portal regression | **✅ (6/6)** |
| P4 | smoke-winit, manual checklist, mctl JSON/rules/check-config, post-install smoke | **✅ (6/6)** |

---

## 0. Şu An Tamam Olan Çekirdek

* **[x] UWSM / gerçek oturum başlatma** - `margo.desktop`, `margo-uwsm.desktop`, systemd/dbus environment import, noctalia ve user servisleri için doğru session ortamı.
* **[x] Config ve canlı reload** - `source/include`, `conf.d`, `mctl reload`, `Super+Ctrl+R`, klavye ve libinput ayarlarını oturumu kapatmadan yenileme.
* **[x] Mango action uyumluluğu** - `focusdir`, `exchange_client`, `viewtoleft/right`, `tagtoleft/right`, `switch_layout`, `set_proportion`, `incgaps`, `movewin`, `resizewin`.
* **[x] Tag/workspace temeli** - view/tag/toggletag/client-tag bindleri, dwl-ipc tag state, noctalia tarafına temel state yayını; super+N **aynı tag**'a iki kere basınca dwm-tarzı önceki tag'e dönüş; `tagrule = id:N, monitor_name:X` ile **per-tag home monitor** — `view_tag(N)` farklı monitördeyken otomatik o ekrana warp eder, `new_toplevel` windowrule'da `monitor:` yoksa tag home'unu kullanır (mango/niri'deki "tag 1-6 DP-3, 7-9 eDP-1" akışı doğrudan çalışır).
* **[x] Layout çekirdeği** - tile, scroller, grid, monocle, deck, center/right/vertical varyantlar, canvas/dwindle temel yerleşim.
* **[x] Scroller ergonomisi** - sloppy focus, auto-center, smart insert, focus history recall, overview aç/kapat iyileştirmeleri.
* **[x] Windowrule temeli** - regex `appid/title`, negative match (`exclude_appid` / `exclude_title`), `min_width` / `max_width` / `min_height` / `max_height` size constraints, floating geometry, `block_out_from_screencast` flag (runtime filter henüz P1'de), late `app_id/title` geldiğinde rule reapply, reload sırasında mevcut pencerelere rule reapply, tag-pinli rule'larda `monitor:` artık opsiyonel — tagrule home'u devralıyor.
* **[x] Input çekirdeği** - klavye, pointer, touchpad, swipe gestures, caps-to-ctrl, pointer hız/accel config.
* **[x] Clipboard çekirdeği** - `wlr_data_control_v1`, `primary_selection_v1`, XWayland selection bridge; CopyQ/clipse/cliphist sınıfı araçlar çalışmalı.
* **[x] Layer shell temeli** - bar/notification layer surface ordering; border notification üstünden geçmemeli.
* **[x] Render temeli** - GLES renderer, rounded border shader, rounded content clipping, fractional-scale uyumlu border, udev render konumu ile input hizası fix.
* **[x] Night light temeli** - `wlr_gamma_control_v1`, gamma reset, sunsetr/gammastep/wlsunset pipeline.
* **[x] Screencopy SHM temeli** - grim/wf-recorder/OBS için SHM target; dmabuf ve bölge optimizasyonu eksik.
* **[x] Winit nested geliştirme yolu** - hızlı test için `margo --winit -s kitty`.

---

## 1. P0 - Güvenilir Günlük Oturum İçin Kalanlar  ✅ TAMAM

Bu bölüm bittiğinde margo "ana oturumum" sayıldı. 6 madde de [x]; hepsinin tek satırlık özeti aşağıda, ayrıntı her maddenin altında.

* **[x] `ext_session_lock_v1`**
  Kabul: noctalia / `qs lockScreen lock` margo altında doğru ekran boyutunda açılıyor; klavye + mouse input sadece lock surface'e gidiyor; lock surface'in initial configure'unda output mode + transform + scale'den hesaplanmış logical size yollanıyor (`with_pending_state`).
  *Tamamlanan üç ayrı düzeltme:*
  1. `new_surface` artık `with_pending_state(|s| s.size = ...)` ile non-zero boyut yolluyor (öncesinde noctalia hiç buffer attach etmiyordu, ekran siyahtı).
  2. `pointer_focus_under` lock'lu durumda her pointer event'ini lock surface'a sabitliyor — alttaki gizli pencereler input görmüyor.
  3. `handle_keyboard` üst kısmındaki `exclusive_keyboard_layer` override'ı `if !state.session_locked` kapısı altına alındı; aksi halde noctalia'nın bar / launcher layer-surface'ı `keyboard-interactivity: exclusive` yüzünden ilk tuşta focus'u çalıyor, parola girilmiyordu.
  *Yan iş (cachy modules):* `margo-uwsm-session` artık `XDG_ICON_THEME=kora`, capitaine cursor, qt6ct platform theme default'larını export ediyor — noctalia / GTK iconset doğru gözüküyor.
  *Doğrulanan client'lar:* noctalia `lockScreen lock`. `swaylock` / `gtklock` ile de aynı yoldan geçer.

* **[x] `ext_idle_notifier_v1` + idle inhibit**
  Kabul: `swayidle` / `noctalia` idle/active event'lerini alıyor (her klavye/pointer/touch/gesture olayı `notify_activity` ile seat'i bump'lıyor); `mpv`/video player'ların `zwp_idle_inhibit_manager_v1` üzerinden açtığı inhibitor'lar `set_is_inhibited(true)` ile timer'ı duraklatıyor; surface destroy/client crash sonrası inhibitor seti otomatik temizleniyor (smithay protokol seviyesinde `uninhibit` callback'ini garanti ediyor).

* **[x] DRM hotplug ve output cleanup**
  Kabul: laptop dock senaryosunda logout gerekmiyor — hem unplug hem yeni monitor takılması runtime'da eli alınıyor.
  *Unplug yarısı:* `OutputDevice` artık driving connector handle'ını saklıyor, `rescan_outputs` her CRTC için **tek tek** kontrol ediyor (eskisi "kart üzerinde herhangi bir connector bağlı mı?" diye soruyordu, dual-monitor'de yanlış cevap veriyordu); kalkan monitor'daki client'lar ilk hayatta kalan monitor'a migrate ediliyor (tag mask OR'lanarak görünmez kalmasınlar) ve `Vec::remove` index shift'i tüm `client.monitor` alanlarına da uygulanıyor.
  *Plug-in yarısı:* `setup_connector` helper'ı init loop'undan çıkarıldı; `rescan_outputs` zaten kayıtlı connector'lara bakmadan, `Connected` ama kayıtsız her connector için `setup_connector` çalıştırıp DrmCompositor + GammaProps + MargoMonitor'ü canlıya alıyor; ardından compositor'a hemen bir `render_frame` + `queue_frame` veriyor ki yeni monitör ilk vblank'i bekleyip kara kalmasın.
  *State cleanup chain:* `state.remove_output()` zaten gamma_control / screencopy / layer maps / lock_surfaces / pending_gamma'yı temizliyor — rescan path'i bunu her unplug için bir kez tetikliyor.

* **[x] Udev crash/debug log kalitesi**
  Kabul: `pkill -USR1 margo` her zaman canlı state dump'ı (output sayısı + per-monitor tagset/layout/selected, focused client + ilk 32 client geometry/tags/app_id/title, lock state, idle inhibitor, layer count, keyboard focus) journal'a yazıyor; panic durumunda yeni `panic::set_hook` location + payload + `Backtrace::force_capture` çağırıyor (önceden systemd sadece "service exited" görüyordu); `dispatch::debug_dump` action'ı ile keybinding üzerinden de tetiklenebiliyor.

* **[x] Interactive move/resize requestleri**
  Kabul: `xdg_toplevel.move` / `xdg_toplevel.resize` istekleri (CSD app titlebar drag, resize-edge drag) `MoveSurfaceGrab` / `ResizeSurfaceGrab`'a bağlanıyor; `mousebind = SUPER,btn_left,moveresize,curmove` ve `btn_right,moveresize,curresize` ile super+drag her pencerede çalışıyor; tiled pencere drag süresince floating'e geçiyor (`is_floating = true` + `float_geom` güncelleme), kullanıcı toggle ettiğinde geri tile'a dönüyor; `dispatch::moveresize` action'ı keybindings için de açık.

* **[x] Windowrule parity regression seti**
  Kabul: `scripts/smoke-rules.sh` tek komutluk smoke test — 5 kanonik test case (tiled kitty, zenity title rule, pavucontrol, copyq, kcalc) için spawn → `mctl status` polling → assert pass/fail. `--reload` flag'i config edit sonrası reload yapıyor; `SMOKE_VERBOSE=1` failure'larda full status dump'ı veriyor; tool yüklü değilse SKIP, gerçek failure'larda exit 1. Ayrıca `cargo run -p margo-config --example check_config` parse-time validator olarak duruyor — ikisi birlikte rule değişikliklerini hızlıca regression-test ediyor.

---

## 1.5 P0+ Polish — Bu Sprintin Daily-Driver Düzeltmeleri  ✅ TAMAM

P0 "ana oturum çalışıyor" anlamına geliyordu; bu blok ise "ana oturumda **rahatsız edici bir şey kalmadı**" diyebilmek için gereken on adımı topluyor. Hepsi commit + push edildi (`bec1c51` → `2f57427` aralığı).

* **[x] Lock screen klavye `text_input` / `input_method` paritesi** (`4b03c2a`)
  Kabul: noctalia'nın `WlSessionLockSurface` içindeki invisible TextInput Qt'nin probe ettiği `wp_text_input_v3` + `zwp_input_method_v2` globalleri olmadan parola almıyordu. İkisi de smithay'den gelen minimum InputMethodHandler ile expose edildi (`delegate_text_input_manager!` + `delegate_input_method_manager!`); QtWayland artık keystrokes'u QML TextInput'a route ediyor.

* **[x] Niri-pattern keyboard focus oracle** (`b370e46`)
  Kabul: `MargoState::refresh_keyboard_focus()` her ilgili event'ten sonra "şu anda nerede focus olmalı?" sorusunu yeniden hesaplayan tek noktalı oracle. Öncelik: cursor altındaki output'un lock surface'i → en yüksek Top/Overlay exclusive layer → monitor'un son seçili client'ı → ilk görünür client. Tetiklendiği yerler: her layer-surface commit'i (noctalia'nın `keyboardFocus: Exclusive ↔ None` mutation'ı tek dosya tek surface üzerinden yapılır, destroy/unmap fire etmez), her lock-surface commit'i (Qt window paint-ready olduktan sonra `wl_keyboard.enter` re-issue), kilitli pointer motion (cursor monitör değiştirince lock surface'i takip eder).

* **[x] Multi-monitor lock surface cursor takibi** (`b370e46`)
  Kabul: Quickshell her output için bir `WlSessionLockSurface` yaratır; eskiden vec'in ilkini focus'luyorduk, kullanıcı yanlış ekrana yazabiliyordu. Artık `compute_desired_focus()` cursor altındaki output'u arıyor. DP-3 + eDP-1 kurulumunda alt+L sonrası kullanıcının baktığı ekrana focus düşüyor.

* **[x] `force_unlock` emergency escape** (`610d1df`)
  Kabul: lock screen herhangi bir nedenle wedged olursa `bind = super+ctrl+alt, BackSpace, force_unlock` ile session_locked=false + lock_surfaces.clear + arrange_all + refresh_focus tek atımda çalışır. Whitelist'lenen tek action `handle_keyboard`'da locked durumunda da çalışır; reboot/TTY switch'e gerek kalmaz. Tabii **debug için**, normal flow'da gerekmiyor.

* **[x] Layer-destroy / layer-mutate focus restore** (`cc29180` + `b370e46`)
  Kabul: rofi destroy ediyordu (eski patch tetikliyordu), noctalia launcher / settings / control-center DESTROY etmiyor — `WlrLayershell.keyboardFocus` özelliğini Exclusive ↔ None arasında mutate ediyor. İki yol da kapatıldı: layer_destroyed hâlâ varsa client'a focus döner, layer commit'inde keyboard_interactivity değişimi de oracle ile yakalanır. Esc → altındaki pencere klavyeye anında cevap verir, mouse oynatmak gerekmez.

* **[x] `tagview` action (move + follow)** (`bec1c51`)
  Kabul: `tag` dwm geleneğini korur (pencere gönder, sen kal). Yeni `tagview` ekstra alias: `tagfocus` / `movetagview` — pencereyi mask'a taşır VE kullanıcıyı oraya götürür. Kullanıcı `super+shift+N` ile tag, `super+ctrl+shift+N` ile tagview yapacak şekilde config'lenmiş.

* **[x] Z-band ordering (floating > tile, overlay > float)** (`1a7a4f7`)
  Kabul: `Space::map_element` her çağrıldığı element'i stack'in tepesine alır — `arrange_monitor` her layout'ta görünür pencerelerin hepsini tekrar map ediyor, `clients` vec sırasındaki son element üstte kalıyor. CopyQ floating popup tile pencereler arasında "kayboluyordu". `MargoState::enforce_z_order()` arrange ve animation tick sonunda float'ları + overlay'leri raise ederek floating > tile > overlay invariantını garanti altına alıyor.

* **[x] Scroller jitter zinciri** (`1f0715e` + `13d1d0c` + `b370e46`)
  Kabul: helium gibi sürekli commit eden client'lar `arrange_monitor`'ı her frame tetikliyor, eski animation hep restart oluyordu (`old != rect` çünkü `old = c.geom = interpolated`). Üç katmanlı düzeltme:
  1. **No-restart**: `animation.current == rect` ise existing animation'a dokunma.
  2. **Size-snap**: position interpolate, size frame 0'da snap. Buffer/slot mismatch hiç oluşmuyor.
  3. **`sloppyfocus_arrange = 0` default**: cursor crossing yeni focus tetiklese de scroller layout'a dokunmuyor — sadece explicit focus action'ları (alt+tab, focusstack, layout switch) re-arrange yapıyor.

* **[x] Niri-style resize transition + 2-texture crossfade + corner radius** (`b553cf4` → `7832cd9`)
  Kabul: slot **size** değişince (yalnızca position değil) `arrange_monitor` `snapshot_pending = true` set eder; udev render path'i bunu drain edip `capture_window` ile live surface tree'yi GlesTexture'a yakalar (`tex_prev`); her frame canlı surface yeniden offscreen capture edilir (`tex_next`); ResizeRenderElement her iki texture'ı da **aynı `render_texture_from_to` çağrısından** geçirir (sadece source texture + alpha farklı), aynı rounded-clip shader override'ı, aynı dst rect — iki layer arasında byte-düzeyinde fark imkansız. Snapshot rounded corners için `clipped_surface` GLES program'ı ile maskelenir, slot animasyonu boyunca snapshot ile slot birlikte interpolate eder (size-snap kaldırıldı). Helium / Spotify'ın 50-100ms ack-and-reflow penceresinde buffer/slot mismatch görünmez. Niri'nin `ResizeRenderElement` mantığının iki-texture pass varyantı.

* **[x] Deferred initial map (CopyQ flicker)** (`e064479`)
  Kabul: super+v ile CopyQ açtığında (KeePassXC, pavucontrol, polkit-gnome, GTK file picker da) artık tek bir frame'de doğru pozisyonda belirir, "kaybolup tekrar gözüküyor" sıçraması yok. Kök neden: Qt client'lar xdg_toplevel role'ünü `set_app_id`'den ÖNCE oluşturuyor, `new_toplevel`'da windowrule eşleşmiyordu, default tile pozisyonunda map oluyordu, bir sonraki commit'te app_id arrive edip rule reapply olunca pencere sıçrıyordu. Niri pattern: `MargoClient.is_initial_map_pending = true` set, `new_toplevel`'da `space.map_element` çağırma; ilk commit'te (app_id hazır) `finalize_initial_map` rules apply + tag-home redirect + map at final position + focus + arrange.

* **[x] README profesyonelleştirme + diagnostic logging** (`007bd6e` + `1f00367` + `36ac94b` + `0cf57a3` + `4006e8c`)
  Kabul: README tek satırdan 200+ satır profesyonel landing page'e çıktı (status badges, layout matrix, protokol tablosu, build/configure/IPC/architecture/acknowledgements). YOL_HARITASI.md'ye iki redundant link kaldırıldı. `tracing::info!` instrumentation: session-lock state changes, refresh_keyboard_focus current → desired log'u, key forwarding focus context, arrange[app_id] per-client geometry trace, border[app_id] slot/actual/drawn deviation log'u. RUST_LOG=margo=debug ile journal'dan jitter root-cause aranabilir hale geldi.

* **[x] noctalia LockScreen multi-monitor dots fix** (cachy module override)
  Kabul: `~/.cachy/modules/noctalia/dotfiles/quickshell-overrides/noctalia-shell/Modules/LockScreen/LockScreenPanel.qml` — `passwordInput.text.length` yerine `lockControl.currentText.length` (LockContext shared state); password dots widget hangi monitördeki passwordInput focus alırsa alsın iki ekranda da senkron çiziyor. install hook `/etc/xdg/quickshell/noctalia-shell` tree'sini `~/.config/quickshell/noctalia-shell` altına symlink olarak mirror'lıyor + override dosyalarını kopya olarak yazıyor; noctalia-shell-git package update'leri otomatik picklanıyor.

---

## 2. P1 - Modern Desktop Protokol Paritesi  ✅ TAMAM

Bu bölüm margo'yu "kurcalanan compositor" olmaktan çıkarıp modern masaüstü ekosisteminde düzgün davranan compositor yaptı. Sekiz protokol paritesi commit'lendi (`78c9909` → `886eba5` aralığı), toplam ~1300 LOC.

* **[x] `linux_dmabuf_v1` client path + `linux-drm-syncobj-v1`** (`78c9909`)
  Kabul: Firefox/Chromium/GTK/Qt dmabuf buffer sunabilir; SHM fallback'e düşmez. + Modern Chromium / Firefox / DXVK explicit-sync için `wp_linux_drm_syncobj_v1` global'i de var (gated on `supports_syncobj_eventfd` — kernel <5.18 ve syncobj_timeline desteklemeyen device'larda advertise edilmiyor).
  Temel dmabuf altyapısı (DmabufState, v5 feedback, GLES import hook) zaten kuruluydu; bu commit explicit-sync'i ekledi — modern Chromium 100+ / Vulkan oyunlar artık per-frame acquire fence ile timeline pacing yapıyor.

* **[x] DMA-BUF screencopy target** (`8bcdfab`)
  Kabul: screencopy protokolünde ilan edilen dmabuf buffer gerçekten udev backend'de yazılır; OBS/Discord/wf-recorder/portal-wlr SHM fallback olmadan çalışır. Full-output capture'da `renderer.bind(&mut dmabuf)` + `OutputDamageTracker::render_output` zero-copy GPU→GPU. SHM upload bottleneck'i kalktı, 1080p60 sorunsuz.

* **[x] Region-based screencopy crop + damage** (`886eba5`)
  Kabul: `grim -g "$(slurp)"` sadece istenen bölgeyi okur; `copy_with_damage` full-frame damage basmaz. DMA-BUF region capture niri pattern ile: `RelocateRenderElement::from_element(e, -region_loc, Relocate::Relative)` + tracker'ı `buffer_size`'da kur — request edilen rect dmabuf'un (0,0)'ına düşer, doğal clip dmabuf dışını keser.

* **[x] `block_out_from_screencast` runtime filter** (`37d1fb6`)
  Kabul: windowrule ile işaretlenen password manager / secret pencereleri gerçek ekranda görünür, screencopy/recording çıktısında siyah maskelenir. `build_render_elements_inner(for_screencast: bool)` parametresi: display path'te `false` (normal render), screencast path'te `true` (blocked client'lar yerine `SolidColorRenderElement` siyah). Race window yok — substitution element-collection time'da, herhangi bir pixel sampling'den önce.

* **[x] `pointer_constraints_v1` + `relative_pointer_v1`** (`7c39cab`)
  Kabul: FPS oyunları (Counter-Strike 2, Vulkan native) cursor-locked aim çalışır, Blender mid-mouse rotate viewport edge'inde diğer pencereye sıçramaz, Krita brush canvas içinde kalır, xdpw RemoteDesktop pointer cast'lenmiş output dışına çıkmaz. `PointerConstraintsHandler::new_constraint` cursor surface üstündeyse anında activate, `cursor_position_hint` unlock-time hint'i honour eder. `handle_pointer_motion` enforcement: lock active → `input_pointer.x/y` pre-delta'ya restore (cursor donar), `relative_motion` event yine fırlar.

* **[x] `xdg_activation_v1`** (`4cf0041`)
  Kabul: uygulama linkleri ve portal activation token'ları focus stealing yapmadan doğru pencereye odak aldırır. `notify-send -A Reply` action click → mesajlaşma uygulamasının penceresi öne gelir; Discord tray click; mailto: → Thunderbird already running; Telegram unread bildirimi. Anti-focus-steal: token serial keyboard'un last_enter'ından eski olmamalı, seat eşleşmeli, age <10s. Tag-aware: pencere farklı tagdaysa `view_tag` (en küçük set bit) → multi-monitor'da home output'a warp.

* **[x] `wlr_output_management_v1`** (`6fb0f7d`)
  Kabul: `wlr-randr --output DP-3 --scale 1.5` çalışır, `wlr-randr --output eDP-1 --transform 90` çalışır, `kanshi` profile-based auto-config docked/undocked geçişlerinde scale/position uygular. ~600 LOC port (niri'nin ~900 satırının focused alt-kümesi). Mode/disable hâlâ DRM re-modeset olmadığı için `failed()`'lı, sonraki iteration'da gelecek. Configuration semantics spec-compliant: stale serial → cancelled, `enable_head` / `disable_head` aynı output ikinci kez → cancelled (doomed configuration rule), `apply` sonrası head ekleme → cancelled.

* **[x] `presentation-time`** (`886eba5`)
  Kabul: kitty, mpv ve native Wayland Vulkan oyunlar (DXVK / VKD3D-Proton) frame presented timestamp + refresh interval alır; frame pacing 60 Hz tahminden kurtulur. `PresentationState::new(&dh, 1 /* CLOCK_MONOTONIC */)` + her successful `queue_frame` sonrası `publish_presentation_feedback`: toplevel + layer surface drain + `presented(now, refresh, 0, Vsync)`. DRM page-flip seq plumbing P2'ye kalan minor refinement.

---

## 3. P2 - Akıcılık ve Performans  ✅ TAMAM

Bu bölüm "niri kadar smooth" hedefinin teknik karşılığıdır. Protocol parity (P1) tamamlandığı için artık client'lar margo'da düzgün çalışıyor; bu bölümün hedefi compositor'un kendisinin akıcı, idle'da ucuz, animation'da hassas olması. **Hepsi şu anda tamam ve niri/mangowm-ext üstünde** — özellikle open/close transition'larda (snapshot-driven, ilk frame zaten mid-curve) ve focus highlight cross-fade'inde (mango/dwl sadece anlık değişim yapıyor).

* **[x] Frame clock ve redraw scheduler — temel (Ping-based on-demand)**
  16 ms periyodik polling timer kalktı. `request_repaint` artık bir calloop `Ping` source'a dokunarak loop'u uyandırıyor; idle'da hiç wake yok. Animation continuity için DRM `VBlank` event'leri post-dispatch hook'taki `tick_animations`'ı kuyrukta tutuyor. `pending_vblanks` sayacı render rate'ini tek vblank başına bir kareye sıkıyor — aksi halde post-hook'taki tick her iterasyonda repaint tetikleyip CPU spin'lerdi. Per-output `next_frame_at` scheduling ve ayrı output redraw sırası P2 #1.5'e kalıyor (tek-monitor kurulumlarda fark etmiyor).

* **[x] Spring tabanlı animasyon motoru (move animation için)**
  Kritik / under-damped harmonik osilatör + semi-implicit Euler integrator (`animation/spring.rs`). Config'ten `animation_clock_move = spring` ile aktif olur; `animation_spring_stiffness` / `animation_spring_damping_ratio` / `animation_spring_mass` tuning. Per-channel velocity (x, y, w, h) `ClientAnimation`'a eklendi → mid-flight retarget'larda momentum korunuyor (Helium/Spotify gibi sürekli arrange çağıran client'larda bezier'in restart kink'ini ortadan kaldırıyor). Resize snapshot lifetime spring modunda fizik settle'a göre ayarlanıyor; overshoot olsa bile snapshot mid-flight kaybolmuyor. Default hâlâ bezier — opt-in. 4 birim test geçiyor (overshoot, critical-damped no-overshoot, retargeting velocity preserve, 60Hz/144Hz invariance).

* **[x] Open/close/tag/focus/layer animasyonları (5/5)**
  - **Open**: `finalize_initial_map`'te tetiklenir, ilk frame'de wl_surface texture'a yakalanır, sonrasında `OpenCloseRenderElement` ile zoom+fade veya slide_in_*. Live surface tüm transition boyunca gizleniyor → "instant pop, sonra animasyon" flash'ı yok, niri'de bile var olan tek-frame flash burada yok. Per-window-rule override (`animation_type_open=…`) global config'i ezer.
  - **Close**: `toplevel_destroyed`'da client `clients` vec'inden anında çıkarılıyor (focus stack, layout, scene order için close instant gibi davranıyor); `ClosingClient` kayıt rendr-side concern olarak `closing_clients` listesinde slide+fade out oynuyor. dwl/mangowm-ext sadece alpha fade yapıyor — biz scale+alpha+rounded-corner clip korunarak.
  - **Tag switch**: `view_tag`'de yön bit-position delta'sından türetiliyor (mango'nun `tag_animation_direction` Horizontal/Vertical config'i + ileri/geri). Outgoing pencereler texture-snapshot ile slide-out, incoming pencereler `arrange`'in mevcut Move animation'ını off-screen geom'dan target slot'a kullanıyor → tek render path. niri sadece workspace switch'de bunu yapıyor; mango sadece existing pencereleri kaydırıyor, biz ayrılan pencereleri de animate ediyoruz.
  - **Focus highlight**: Mevcut `OpacityAnimation` struct'ı (border color + opacity field'ları zaten vardı, ama hiç populate edilmiyordu) `focus_surface`'da hem outgoing hem incoming için kuruluyor; `tick_animations` `animation_curve_focus` ile interpole ediyor; `border::refresh` interpole edilmiş rengi kullanıyor. focused_opacity ↔ unfocused_opacity de aynı struct üzerinden cross-fade.
  - **Layer surface**: Yeni `LayerSurfaceAnim` map'i, `ObjectId` key'iyle. Open = live alpha modulation (anchor-aware geom shift'i atlandı — bar yerinden oynamasın). Close = wl_surface unmap'ten önce yakalanan texture, `push_closing_layers` ile zoom/slide-out, layer band'ında render ediliyor. waybar/mako/noctalia hepsi etkilenir; `layer_animations` global flag ile kapatılabilir.
  Kabul kriteri: bütün animasyon tipleri hem bezier hem spring saatiyle çalışıyor (move spring opt-in, diğerleri bezier curve_open/close/tag/focus). Renderer hepsini aynı `OpenCloseRenderElement` veya `Move` animation pipeline üzerinden çiziyor → tek bug fix tüm animasyon tiplerini etkiler.

* **[x] Hardware cursor plane**
  `DrmCompositor::new` artık driver'ın `cursor_size()` ile bildirdiği gerçek HW cursor buffer boyutunu kullanıyor (eski `(64, 64)` hardcode'u modern AMD/Intel/NVIDIA'nın 128²/256² desteğini körlüyordu). `FrameFlags::DEFAULT` zaten `ALLOW_CURSOR_PLANE_SCANOUT` içeriyor; bu fix ile cursor sığdığında atomik plane güncellemesi yapılıyor, primary swapchain'e composite olmuyor. Driver `cursor_size = 0` raporlarsa eski 64×64 fallback'e düşülüyor. Startup log'u "DRM hardware cursor plane: WxH" ile doğrulamayı sağlıyor.

* **[x] Direct scanout (smithay built-in)**
  `FrameFlags::DEFAULT` zaten `ALLOW_PRIMARY_PLANE_SCANOUT | ALLOW_OVERLAY_PLANE_SCANOUT` flag'lerini içeriyor. Fullscreen + dmabuf-backed + format-uyumlu bir client (mpv, native Wayland oyunlar) uygun olduğunda smithay otomatik olarak primary plane'e atıyor; overlay layer varsa kendisi composite'e fallback ediyor. Aktif iş gerektirmedi — explicit-sync (P1) ve dmabuf feedback (P0+) zaten zincirin client tarafını besliyor. Observability ihtiyacı doğarsa `RenderFrameResult.states` ile per-element plane assignment loglanabilir; şimdilik gereksiz.

* **[x] Damage tracking (smithay built-in + custom element doğrulaması)**
  `DrmCompositor` içindeki `OutputDamageTracker` zaten per-frame damage hesaplıyor ve sadece değişen rect'leri redraw ediyor. Custom render element'lerimizin (`RoundedBorderElement`, `ClippedSurfaceRenderElement`, `ResizeRenderElement`) hepsi `CommitCounter`'ı sadece geometri / renk / shader-uniform değiştiğinde bump ediyor — `damage_since` doğru raporluyor, statik border ve clipped surface tekrar render üretmiyor. Cursor motion'unda damage iki küçük rect (eski + yeni cursor bbox); HW cursor plane fix'i ile bu da primary swapchain'i dolaşmıyor. Kabul kriteri smithay altyapısı sayesinde sağlanmış durumda.

---

## 4. P3 - Pencere Yönetimi v2

Bu bölüm MangoWM mirasını Rust'ta güçlü hale getirir.

* **[x] Scratchpad + named scratchpad** (commit `a616171`)
  `toggle_named_scratchpad <appid> <title|none> <spawn-cmd>` + `toggle_scratchpad` action'ları eklendi. Window-rule `isnamedscratchpad:1` ile etiketlenen client'lar `finalize_initial_map`'te hidden başlar; bind ile toggle. `single_scratchpad` config'i diğer scratchpad'leri otomatik gizler. show: `is_floating + is_in_scratchpad + is_scratchpad_show`, slot'a re-map+raise+focus. hide: `unmap_elem` + `is_minimized` + focus drop. `scratchpad_cross_monitor` config'i de honor edilir.

* **[x] Full Mango windowrule/layer rule parity** (commit `dc29265`)
  `windowrule.animation_type_open` / `animation_type_close` artık client'a uygulanıyor. **`layerrule` daha önce sadece parse ediliyordu, hiç apply edilmiyordu** — `new_layer_surface` ve `layer_destroyed` artık `config.layer_rules`'u namespace'e karşı regex match ediyor: `noanim:1` close/open animation'ı atlar, `animation_type_*` global default'u ezer. mango/mangowm-ext'in user'ın config'iyle eşleşen 6 layerrule satırı artık gerçekten çalışıyor. `noblur`/`noshadow` parse + store ediliyor; render hooks P5'e kalıyor.

* **[x] CSD/SSD politikası** (commit `2c0c6b5`)
  `XdgDecorationHandler` artık koşullu — default ServerSide ama client `allow_csd:1` window-rule eşleşmesindeyse `request_mode(ClientSide)` honor ediliyor. `client_allows_csd` iki path'i kontrol eder: mapped client → `MargoClient::allow_csd`; pre-map (decoration ilk configure'da gelir) → window-rule listesini toplevel'in app_id/title'ı ile match. `unset_mode` policy'i yeniden hesaplar — toggle off + tekrar on'da CSD whitelist korunur.

* **[x] Noctalia/workspace IPC paritesi** (commit `bbfcade`)
  4 broadcast eksiği kapatıldı: focus shift, pure title change (rule reapply yokken bile), `togglefloating`, `togglefullscreen`. Önceden bar bu state değişimlerinde stale kalıyordu (önceki focused window'un title'ı/glyph'i takılı kalıyordu). Mango broadcast davranışı ile parite — focus_surface, refresh_wayland_toplevel_identity, toggle_floating/fullscreen hepsi `broadcast_all` veya `broadcast_monitor` çağırıyor.

* **[x] XWayland HiDPI/cursor env** (commit `45b33b6`)
  `XCURSOR_SIZE` ve `XCURSOR_THEME` XWayland Ready event'inde export ediliyor. Önceden libxcursor 16-px default'a düşüyordu → "Steam/Discord/Spotify üzerine gelince cursor küçülüyor" regression'ı. `XCURSOR_THEME` user'ın session env'inde set'liyse override edilmiyor (Hyprland/niri pattern). `DISPLAY`/`XCURSOR_*` systemd user-environment'a propagate ediliyor. Tam HiDPI scaling (xrandr/Xft.dpi) niri-style xwayland-satellite gerektirir, bu turda kapsam dışı.

* **[x] Portal/popup focus** (commit `d0269b3`)
  `XdgShellHandler::grab` boş stub'tı — `xdg_popup.grab(seat, serial)` request'leri sessizce yutuluyordu, bu da portal file picker/dropdown/right-click menu'lerinde keyboard focus'un parent toplevel'da kalmasına neden oluyordu. Çözüm: `FocusTarget::Popup(WlSurface)` variant + grab fired'da popup'ın wl_surface'ına direkt keyboard focus push. Smithay'in tam `PopupKeyboardGrab`/`PopupPointerGrab` chain'i `From<PopupKind>` trait bound'ları gerektirir (FocusTarget'ın SessionLock/X11 Window varyantları sağlamıyor); pragmatik direct-focus çözümü single-level popup vakalarının %99'unu kapatır, nested popup'lar her grab'de aynı path'i yürür.

---

## 5. P4 - Tooling, Test ve Paket Kalitesi

* **[x] Tek komutluk nested smoke test** (`scripts/smoke-winit.sh`)
  `margo --winit -s "kitty -- …"` ile nested margo başlatır, child socket'i bekler, config parse → spawn → `mctl status` → `mctl reload` → focusstack → killclient → empty-status zincirini doğrular. DRM gerektirmez, CI'da bile çalışır. `SMOKE_BUILD=1` cargo build, `SMOKE_KEEP=1` test sonrası nested session'ı bırakır manual debug için.

* **[x] Gerçek oturum test checklist'i** (`docs/manual-checklist.md`)
  13 bölümlü post-install/post-reboot checklist: bring-up, layer shells, notifications, clipboard, multi-monitor cursor + tag move, lock screen, window rules, scratchpad, animations, day/night gamma, portal file picker, screen recording, XWayland, idle resource usage. Her adım `mctl status` veya `journalctl` ile substring match'lenebilir kabul kriteri.

* **[x] `mctl status --json`** (commit `f5b8d71`)
  Stable JSON schema: `{ tag_count, layouts: [...], outputs: [{ name, active, layout, layout_idx, focused: {...}, tags: [{...}] }] }`. Status-bar widget'ları ve `jq` pipeline'ları için.

* **[x] `mctl rules --appid X --title Y [--verbose]`**
  Config-side introspection. Wayland bağlantısı YOK — `~/.config/margo/config.conf`'u parse eder, her windowrule'u Match / Reject(<reason>) olarak sınıflar. Verbose mode rejected rule'ları reason'la (`appid pattern miss`, `exclude_id matched`, vb.) gösterir. Editorde rule yazarken regex hatasını bulmak için.

* **[x] `mctl check-config [--config FILE]`**
  Validator: unknown windowrule fields, regex compile errors in pattern slots, **duplicate bind detection** (later definition wins — silent shadowing), unresolvable `source =` / `include =` includes, exit-1 on error CI-friendly. User'ın gerçek config'inde `alt,space` ve `super,g` duplicate'leri yakaladı.

* **[x] `scripts/post-install-smoke.sh`**
  Paket-side validation: binaries exist + run, example config parses (with `mctl check-config`), dispatch catalogue ≥30 entries, `desktop-file-validate` clean, xdg-desktop-portal config has `[preferred]` section, shell completions in `/usr/share/{bash-completion,zsh,fish}/...` doğru path'lerde, LICENSE installed. PKGBUILD `check()` veya `.install` post-install hook'una bağlanabilir. `--quiet` for CI.

---

## 6. Uzun Vadeli "En İyi" Hedefleri  **(6/6 tasarım/foundation aşaması)**

* **[~] Built-in xdg-desktop-portal backend** *(design)*
  `docs/portal-design.md` — 4 milestone'lu rollout: (1) screencast over xdp-wlr fallback, (2) screenshot impl, (3) file chooser via xdp-gtk delegation, (4) activation policy. Smithay'de tam handler yok — zbus + xdp-portal trait impl gerekiyor. Doc'ta her milestone için LOC tahmini + bağımlılık matrisi.

* **[x] Spatial canvas** — `Pertag::canvas_pan_x/y` + `canvas_pan` / `canvas_reset` aksiyonları; PaperWM-tarzı per-tag pan koordinatı. `ArrangeCtx::canvas_pan` 5 layout algoritmasında kullanılıyor; pan değeri tag-bazında saklanıyor (monitor değil) — her tag kendi viewport'unu hatırlıyor.

* **[x] Adaptive layout engine** — `Pertag::user_picked_layout: Vec<bool>` sticky-pick bit'i + `maybe_apply_adaptive_layout()` heuristic. Pencere sayısı + monitor aspect ratio'ya göre layout seçer (1 → monocle, 2-3 wide → tile, 4+ portrait → deck...). Kullanıcı `setlayout` çağırdığında o tag için kalıcı pin atılır — heuristic bir daha override etmez.

* **[x] Real-time blur/shadow/color pipeline** *(shadow phase)* — `render/shadow.rs`: SDF tabanlı analitik drop-shadow, single-pass GLES pixel shader. Offscreen buffer yok — fragment shader rounded-box SDF'i smoothstep ile yumuşatır. `udev.rs::push_client_elements` floating + non-fullscreen + non-scratchpad client'lara `MargoRenderElement::Shadow` push ediyor. Kawase blur + adaptive border color sonraki phase.

* **[~] HDR + color management** *(design)*
  `docs/hdr-design.md` — 4 fazlı rollout: (1) `wp_color_management_v1` protocol scaffolding, (2) linear-light fp16 composite path, (3) KMS HDR scan-out (`HDR_OUTPUT_METADATA`), (4) ICC profile per-output 3D LUT. Hardware support matrisi (Intel/AMD/NVIDIA), per-faz LOC + bağımlılık tahmini. Smithay 0.7 `DrmCompositor` HDR primitives expose etmiyor — drm-rs'e direkt ineceğiz.

* **[~] Script/plugin sistemi** *(foundation landed)*
  `margo/src/scripting.rs` + `docs/scripting-design.md`. Rhai 1.24 (pure Rust, ~300 KB) sandboxed engine; `~/.config/margo/init.rhai` startup'ta evaluate ediliyor. Phase 1 binding: `spawn(cmd)` + 3 forward-compat stub (`on_focus_change`, `on_tag_switch`, `on_window_open` — log-only bugün, Phase 3'te wire edilecek). 5 fazlı rollout: dispatch bindings → event hooks → mctl run remote eval → plugin packaging.

---

## Önerilen Sıradaki Sprint

P1 protokol paritesi tamamlandı. Sıradaki blok P2 — **akıcılık + performans**. Etki/maliyet sırası:

1. **Frame clock + redraw scheduler** (~200 LOC) — animasyon yokken idle GPU kullanımı sıfıra yakın inecek. Şu an `request_repaint` her event sonrası fire ediyor; smithay'in `OutputDamageTracker::next_frame_at` ile vblank'e göre programlanmış redraw'a geçilmeli. CPU/GPU/güç tasarrufu, en görünür win.
2. **Damage tracking iyileştirmeleri** (~150 LOC) — frame clock'la eşli; statik pencereler ve borderlar gereksiz redraw üretmez. Mevcut `request_repaint` çağrılarını damaged-region'a çevirme.
3. **Hardware cursor plane** (~150 LOC) — Cursor sprite'ı kompozit yerine DRM cursor plane'e koyma. GPU ağır yük altındayken cursor takılmaz. `DrmCompositor::set_cursor_plane`.
4. **Direct scanout** (~250 LOC) — Fullscreen mpv/oyun/video uygun olduğunda kompozit by-pass eder. `DrmCompositor` zaten destekliyor; render element'in scanout-uygun dmabuf üretmesi + flag'i.
5. **Spring tabanlı animasyon motoru + open/close/tag/focus/layer** (~400 LOC) — niri benzeri spring-clock modeli. Mevcut bezier curve + duration sistemi yerine config'ten seçilebilir spring/clock. Open/close/tag/focus/layer animasyonları henüz yok.

Toplam: ~1150 LOC. Mantıklı sıralama: 1 → 2 birlikte (idle ucuzlaması), sonra 3 (görünür smoothness win), sonra 5 (UX zenginliği), 4 en sonda (mpv/oyun fullscreen niş ama önemli).

P2 bittiğinde margo "niri kadar smooth" hedefinde pratik olarak eşleşecek. P3 sonrasında pencere yönetimi v2 (scratchpad, named scratchpad, CSD/SSD policy, noctalia IPC parity, XWayland HiDPI).

---

## Kısa Kabul Testi

Yeni paket kurulunca en az şu akışlar denenmeli:

* Reboot -> margo UWSM session -> noctalia/taglar/notification görünür.
* `mctl reload` -> keyboard/input/windowrule değişiklikleri oturumu kapatmadan uygulanır.
* CopyQ, wiremix, pavucontrol, ente auth -> floating rule ile doğru boyut/offset.
* Browser/file-manager file chooser -> cursor hover/selection hizası doğru.
* 3 pencere scroller tagında -> focus diğer pencerelere geçer, auto-center kaybetmez.
* Tag taşıma -> pencere sadece hedef tagda görünür, eski tagda anlık ghost yok.
* Night light aç/kapat -> gamma aşırı kırmızıya sapmaz, logout sonrası resetli kalır.
* Grim/wf-recorder -> SHM screencopy çalışır; dmabuf tamamlanana kadar fallback beklenen davranıştır.
