# Margo Codebase Değerlendirmesi

> **⚠️ Tarihsel snapshot (superseded).** Bu dosya, 2026-05-17'de yapılmış tek
> seferlik bir denetim turunun kaydıdır. Aşağıdaki tüm metrikler (crate / LOC /
> test sayıları ve bulgular) **o tarihe aittir ve güncel değildir** — ör. o gün
> 39 crate / 133K LOC idi, bugün `scripts/metrics.sh` canlı sayımı çok daha
> yüksek. Kod kalitesi için **canlı kaynak**:
> [`docs/code-quality-roadmap.md`](docs/code-quality-roadmap.md) (yaşayan yol
> haritası) + [`scripts/metrics.sh`](scripts/metrics.sh) (LOC/crate/test/unsafe
> anlık sayım). Bu dosya yalnızca o turun neyi neden değiştirdiğinin tarihsel
> kaydı olarak tutuluyor.

**Tarih**: 2026-05-17
**Kapsam**: 39 workspace crate, 585 Rust dosyası, 133,431 LOC.
**Build durumu**: ✓ `cargo check --workspace` temiz.
**Test durumu**: ✓ mshell-launcher 90 test geçer, margo/layout 38 snapshot test.
**Clippy**: ~90 uyarı (37'si `--fix` ile otomatik düzelir).

---

## ✅ Bu turun düzeltmeleri

| # | Bulgu | Durum | Commit |
|---|---|---|---|
| C1 | matugen `flatten()` sonsuz döngü | ✅ DONE | `bbe9f58` |
| C2 | CLAUDE.md güncellenmesi | ✅ DONE (yerel — `.gitignore`'da) |
| H3 | clippy `--fix` sweep | ✅ DONE (37 öneri uygulandı) | `4c2dadf` |
| H2 | margo/session.rs 21 unwrap | ❌ FALSE POSITIVE — hepsi test kodunda |
| M4 | compositor.rs panic | ❌ TASARIM — Smithay trait invariant'ı |
| L1 | /tmp paths (mlock + launcher) | ✅ DONE — XDG_RUNTIME_DIR fallback | `01933b3` |
| L2 | wayle-hyprland 5 satır yorum | ✅ DONE — 1 satıra indirildi | `01933b3` |
| M5 | dashbord.png + dashbord1.png | ✅ DONE — silindi + `*.png` gitignore'da | `01933b3` |
| **H1** | menu_settings.rs 4041 LOC refactor | ✅ **DONE — 4041 → 394 LOC (-3373 satır)** | `a71aeb4` |
| **H2** | Production unwrap audit | ⚠️ **REVISED — büyük ölçüde yanlış alarm** | — |

**H2 ikinci tur audit sonucu**:
- `margo/session.rs` 21 unwrap → hepsi test kodu (tespit edilmişti)
- `mshell-config/atomic_write.rs` 13 unwrap → hepsi test kodu
- `mshell-common/notification.rs` 10 unwrap → `parse("const_str").unwrap()` + `format(valid_fmt).unwrap()` — fail-impossible by construction
- `wallpaper_menu_widget.rs` 15 unwrap → çoğu `Mutex::lock().unwrap()` (single-threaded GTK, poison yok) + GTK `downcast_ref().unwrap()` (framework garantisi)
- `mutter_screen_cast.rs` 16 unwrap → benzer pattern (Mutex + internal-generated OwnedObjectPath)
- `pw_utils.rs` 11 unwrap → PipeWire pod parsing where format negotiation already validated type (line 428)
- `screenshot/capture.rs` 4 `min/max().unwrap()` → empty check yapılıyor 5 satır önce; sadece `.expect("outputs non-empty")` ile dokümante edildi

**Sonuç**: 265 prod unwrap rakamı flat grep'ten geldi. Case-by-case 80%'i framework/idiom; sistemik risk yok. Geniş sweep değer yaratmaz, scattered tekil instances zaten case-by-case düzeltiliyor.

**M2 ikinci tur audit sonucu**: `mshell_config::Config` ve `margo_config::Config` ikisini de import eden **tek bir dosya bile yok** (display_settings.rs ikisini import ediyor ama farklı tipler için — `margo_config::TwilightMode` + `mshell_config::atomic_write`). İsim çakışması teorik, pratik yok. Rename'in 126 file blast radius'una karşı yararı sıfır. **#189 kapatıldı (CANCELLED)**.

**Kalan büyük iş (#188)**:
- `#188` margo/state.rs bölme (M1) — zaten kısmen bölünmüş (`state/{handlers,animation_tick,dispatch,...}` modülleri var). Geri kalan 3484 LOC `MargoState` impl block — daha fazla extract için compositor internal'larıyla yakın çalışma gerek; yarım gün+, ayrı session'da tek başına.

---

## 🔴 CRITICAL — Aktif bug, hemen düzeltilmeli

### C1. `matugen.rs` — sonsuz döngü riski

**Dosya**: `mshell-crates/mshell-matugen/src/matugen.rs:35` ve `:41`

```rust
for line in self.reader.lines().flatten() { ... }
for line in BufReader::new(stderr).lines().flatten() { ... }
```

`flatten()` bir `Result` iteratöründe sürekli `Err` üretiliyorsa **sonsuza kadar döner** (clippy uyarısı: `flatten()` will run forever). matugen alt-proses I/O hatası verirse mshell asılır.

**Düzeltme**: `.map_while(Result::ok)` ile değiştir.

### C2. `CLAUDE.md` tamamen yanlış

**Dosya**: `CLAUDE.md:1-10`

> "The C codebase lives in `src/` and `mmsg/`; the Rust rewrite lives in `rust/`."

Proje artık tamamen Rust (Smithay). Ne `src/` var, ne `rust/`, ne `mmsg/`. `margo/` crate'i kendi başına compositor. Gelecek session'larda her yeni Claude yanlış yönlendiriliyor.

**Düzeltme**: Mevcut workspace layout'una göre yeniden yaz.

---

## 🟠 HIGH — Kullanıcıyı etkileyen tech debt

### H1. `menu_settings.rs` — 4041 satırlık god file

**Dosya**: `mshell-crates/mshell-settings/src/menu_settings/menu_settings.rs`

17 menü × (4 alan + 4 setter + 4 effect + 4 update arm + view bloğu) = ~250 satır boilerplate × 17. Yeni menü eklerken her seferinde ~250 satır kopyala-yapıştır (Dashboard eklerken bizzat yaptık). Bir alan kaçırınca compile error.

**Düzeltme seçenekleri**:
- `MenuKind` enum + tek `MenuSettingsRow` widget'ı + dispatch (mevcut `widget_menu_settings.rs` zaten bu yaklaşımı kullanıyor).
- `define_menu_settings_panel!(dashboard, "Dashboard Menu")` makrosu — 250 satırı 1 satıra düşürür.

### H2. Production kodunda 265 `unwrap()`

**Top 10 hotspot**:
- `margo/src/session.rs` — 21
- `margo/src/dbus/mutter_screen_cast.rs` — 16
- `mshell-crates/mshell-frame/src/menus/menu_widgets/wallpaper/wallpaper_menu_widget.rs` — 15
- `mshell-crates/mshell-config/src/atomic_write.rs` — 13
- `margo/src/screencasting/pw_utils.rs` — 11
- `mshell-crates/mshell-common/src/notification.rs` — 10
- `margo-config/src/parser.rs` — 10

session.rs'te 21 unwrap = ekran kilitlenmesi / D-Bus hatası → compositor crash. mutter_screen_cast'taki 16 unwrap = screencast başarısız olunca compositor düşer.

**Düzeltme**: Compositor (`margo/`) ve D-Bus handler'larında `?` veya `match … Err(e) => warn!(…)`. UI widget'larında daha az kritik.

### H3. `mshell-frame` — 42 clippy uyarısı

Tek crate'te 42 uyarı:
- 14 "module has same name as containing module" (örn. `menus/menus.rs`)
- 11 "all variants have postfix `Changed`"
- 7 "very complex type" (Vec<Box<dyn ...>> yapıları)

**Düzeltme**: `cargo clippy --fix -p mshell-frame -- -W clippy::all` 5 öneriyi otomatik uygular. Geri kalanlar elle.

### H4. Smithay git rev'e pinli

**Dosya**: `Cargo.toml:170-173`

```toml
[workspace.dependencies.smithay]
git = "https://github.com/Smithay/smithay.git"
rev = "ff5fa7df392cecfba049ffed55cdaa4e98a8e7ef"
```

Smithay henüz crates.io'da değil → build reproducibility GitHub upstream remote'unun yaşamasına bağlı.

**Düzeltme**: Smithay'in stabilize olmasını bekle veya vendor (yerel kopya) tut. Şimdilik etki düşük ama dokümante etmek lazım.

---

## 🟡 MEDIUM — Mimari ve sürdürülebilirlik

### M1. 24 dosya >800 LOC

| Dosya | LOC |
|---|---|
| `menu_settings.rs` | 4041 |
| `margo/src/state.rs` | 3484 |
| `margo/src/backend/udev/mod.rs` | 3132 |
| `mctl/src/bin/mctl.rs` | 2883 |
| `mshell-frame/src/frame.rs` | 1844 |
| `margo-config/src/parser.rs` | 1697 |
| `margo/src/screencasting/pw_utils.rs` | 1619 |
| `mshell-settings/src/display_settings.rs` | 1398 |

**Düzeltme**: state.rs'i `state/{output,seat,window,ipc}.rs` olarak böl. udev/mod.rs'i `udev/{init,session,device,render}.rs`'e ayır.

### M2. İki ayrı config crate karışıklığı

- `margo-config/` (top-level, compositor `.conf` parser, 1697 + 1143 LOC)
- `mshell-crates/mshell-config/` (YAML, shell)

İsim aynı (`Config`, `Menu`, `Position`); hangi olduğunu anlamak için path'e bakmak gerekiyor. mctl.rs ikisini birden kullanıyor.

**Düzeltme**: `margo-config` → `margo-compositor-config`, `mshell-config` → `mshell-shell-config`.

### M3. 28 mshell crate — bazıları çok zayıf

`mshell-logging`, `mshell-sounds` gibi 1-2 fonksiyonluk crate'ler var. Crate boundary fayda < derleme süresi maliyeti.

**Düzeltme**: <300 satırlık crate'leri `mshell-common`'a veya `mshell-utils`'a birleştir. 28 → ~15 crate.

### M4. 4 `panic!()`/`todo!()` üretim dosyalarında

- `mshell-style/build.rs` — 3 panic (SCSS derleme hatası, kabul edilebilir)
- `mshell-clut-gen/src/main.rs` — 1 panic
- `margo/src/state/handlers/compositor.rs` — 1 panic (**compositor handler → tüm shell crash**)

**Düzeltme**: compositor.rs'teki panic'i error log + skip ile değiştir.

### M5. `dashbord.png` + `dashbord1.png` repo root'ta

Git status'te `untracked`, tipo var (`dashbord` → `dashboard`). CI'de yanlışlıkla commit edilebilir.

**Düzeltme**: `.gitignore`'a `*.png` ekle veya temizle.

### M6. `Changed` postfix enum smell

`MenuSettingsInput::QuickSettingsWidgetListChanged` gibi enumlarda variant'ların yarısı `Changed` ile bitiyor.

**Düzeltme**: Nested enum (`MenuSettingsInput::QuickSettings(QuickSettingsEvent)`).

---

## 🟢 LOW — Polish

### L1. Race-prone /tmp paths

- `mlock/src/main.rs:61` → `/tmp/mlock-debug.log` (sabit isim, çoklu-user'da çakışır)
- `mshell-launcher/src/{history,frecency}.rs` → `/tmp/margo_launcher_*.json` (XDG_CACHE_HOME fallback)

`mshell-gamma/src/wayland.rs:248` zaten `mkstemp` kullanıyor — güvenli.

**Düzeltme**: `tempfile` crate'i veya `$XDG_RUNTIME_DIR/$UID/` kullan.

### L2. Yorum satırlarında ölü dep referansı

`Cargo.toml:148-152` — `wayle-hyprland` 5 satır yorum açıklamasıyla "geri sızmasın diye" duruyor. Niyet temiz ama 5 satır gürültü.

### L3. Test kapsamı dengesiz

| Crate | Test |
|---|---|
| mshell-launcher | 90 ✓ |
| margo/layout | 38 |
| margo/animation | 9 |
| margo/state | yok |
| mshell-frame | yok |
| mshell-settings | yok |

`margo/state.rs` (3484 LOC) için test yok — kritik.

### L4. CHANGELOG.md 117K

Çok büyük. Otomatik tutuluyor olabilir ama text editör yavaşlatır.

### L5. `module_inception` 14 yer

`menus/menus.rs`, `bars/bar_widgets/bar_widgets.rs` gibi yapılar — Rust idiom değil.

---

## 📊 Özet metrikler

| Metrik | Değer |
|---|---|
| Workspace üyesi | 39 |
| Rust dosyası | 585 |
| Toplam LOC | 133,431 |
| Build durumu | ✓ temiz |
| Test durumu | ✓ 90+ geçiyor |
| Clippy uyarı | ~90 (37 fixable) |
| Üretim `unwrap()` | 265 |
| Üretim `panic!/todo!` | 4 (1'i kritik) |
| `unsafe` blok | ~15 (çoğu libc, gerekli) |
| Dosya >800 LOC | 24 |
| TODO/FIXME | 11 |

---

## 💡 Eylem sırası (etki/maliyet)

| # | İş | Maliyet | Etki |
|---|---|---|---|
| 1 | C1 matugen flatten | 10dk | Sonsuz döngü → crash önler |
| 2 | C2 CLAUDE.md güncelle | 30dk | Gelecek session'lar doğru |
| 3 | H3 `cargo clippy --fix` | 5dk | 37 uyarı otomatik düşer |
| 4 | M4 compositor.rs panic | 15dk | Crash riski azalır |
| 5 | H2 margo/session.rs unwrap | 1 saat | Compositor stability |
| 6 | H1 menu_settings makro | 2 saat | Gelecek menü eklemeleri 250→1 satır |
| 7 | M1 state.rs böl | yarım gün | Merge conflict azalır |
