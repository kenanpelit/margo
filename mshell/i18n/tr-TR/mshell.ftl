## Türkçe (tr-TR) — Eksik anahtarlar Fluent fallback'i ile en-US'tan
## gelir, burada sadece kullanıcının sık gördüğü etiketleri çeviriyoruz.

app-name = mshell

## Updates module
updates-up-to-date = Güncel ;)
updates-available =
    { $count ->
        [one] { $count } Güncelleme mevcut
       *[other] { $count } Güncelleme mevcut
    }
updates-button-update = Güncelle
updates-button-check-now = Şimdi kontrol et

## Media player module
media-player-not-connected = MPRIS servisine bağlı değil
media-player-heading = Oynatıcılar
media-player-loading-cover = Kapak yükleniyor...
media-player-no-title = Başlık yok
media-player-unknown-artist = Bilinmeyen sanatçı
media-player-unknown-album = Bilinmeyen albüm

## Password / network connection dialog
password-dialog-open-network-title = Açık ağ
password-dialog-authentication-required-title = Kimlik doğrulama gerekli
password-dialog-open-network-warning =
    "{ $ssid }" açık bir ağ. Bu bağlantı üzerinden gönderilen veriler başkaları tarafından görülebilir.
    Yine de bağlanmak istiyor musunuz?
password-dialog-insert-password = { $ssid } ağına bağlanmak için parola girin
password-dialog-cancel = İptal
password-dialog-confirm = Onayla

## OSD
osd-airplane-toggle =
    { $state ->
        [on] Uçak modu açıldı
       *[off] Uçak modu kapatıldı
    }
osd-idle-inhibitor-toggle =
    { $state ->
        [on] Boşta engelleyici açıldı
       *[off] Boşta engelleyici kapatıldı
    }

## Settings — shared
settings-scanning = Taranıyor...
settings-more = Daha fazla
settings-section-connectivity = Bağlantı
settings-section-system = Sistem
settings-section-custom = Özel

## Settings — network
settings-network-wifi = Wi-Fi
settings-network-vpn = VPN
settings-network-vpns-connected =
    { $count ->
        [one] { $count } VPN bağlı
       *[other] { $count } VPN bağlı
    }
settings-network-airplane-mode = Uçak modu
settings-network-nearby-wifi = Yakındaki Wi-Fi

## Settings — bluetooth
settings-bluetooth = Bluetooth
settings-bluetooth-devices = Bluetooth cihazları
settings-bluetooth-known-devices = Bilinen cihazlar
settings-bluetooth-available = Kullanılabilir
settings-bluetooth-pair = Eşleştir
settings-bluetooth-no-devices = Cihaz bulunamadı
settings-bluetooth-connected-count =
    { $count ->
        [one] { $count } cihaz
       *[other] { $count } cihaz
    }

## Settings — power
settings-power-suspend = Askıya al
settings-power-hibernate = Hazırda beklet
settings-power-reboot = Yeniden başlat
settings-power-shutdown = Kapat
settings-power-logout = Oturumu kapat
settings-power-calculating = Hesaplanıyor...
settings-power-full-in = { $duration } içinde dolar
settings-power-empty-in = { $duration } içinde biter
settings-power-profile-balanced = Dengeli
settings-power-profile-performance = Performans
settings-power-profile-power-saver = Güç tasarrufu

## Settings — idle inhibitor
settings-idle-inhibitor = Boşta engelleyici

## Tempo / weather module
tempo-feels-like = Hissedilen { $value }{ $unit }
tempo-humidity = Nem
tempo-wind = Rüzgâr

## Weather conditions
weather-clear-sky = Açık
weather-mainly-clear = Çoğunlukla açık
weather-partly-cloudy = Parçalı bulutlu
weather-overcast = Kapalı
weather-fog = Sisli
weather-fog-rime = Kırağılı sis
weather-drizzle-light = Hafif çisenti
weather-drizzle-moderate = Orta çisenti
weather-drizzle-dense = Yoğun çisenti
weather-drizzle-freezing-light = Hafif donan çisenti
weather-drizzle-freezing-dense = Yoğun donan çisenti
weather-rain-slight = Hafif yağmur
weather-rain-moderate = Orta yağmur
weather-rain-heavy = Şiddetli yağmur
weather-rain-freezing-light = Hafif donan yağmur
weather-rain-freezing-heavy = Şiddetli donan yağmur
weather-snow-slight = Hafif kar
weather-snow-moderate = Orta kar
weather-snow-heavy = Şiddetli kar
weather-snow-grains = Kar taneleri
weather-rain-showers-slight = Hafif yağmur sağanağı
weather-rain-showers-moderate = Orta yağmur sağanağı
weather-rain-showers-violent = Şiddetli yağmur sağanağı
weather-snow-showers-slight = Hafif kar sağanağı
weather-snow-showers-heavy = Şiddetli kar sağanağı
weather-thunderstorm = Hafif/orta gök gürültüsü
weather-thunderstorm-hail-slight = Hafif dolulu gök gürültüsü
weather-thunderstorm-hail-heavy = Şiddetli dolulu gök gürültüsü
weather-unknown = Bilinmeyen hava durumu

## Notifications module
notifications-heading = Bildirimler
notifications-empty = Bildirim yok
notifications-group-count = { $count } yeni

## System info module
system-info-heading = Sistem bilgisi
system-info-cpu-usage = CPU kullanımı
system-info-memory-usage = Bellek kullanımı
system-info-swap-memory-usage = Swap kullanımı
system-info-swap-indicator-prefix = swap
system-info-temperature = Sıcaklık
system-info-disk-usage = Disk kullanımı { $mount }
system-info-ip-address = IP adresi
system-info-download-speed = İndirme hızı
system-info-upload-speed = Yükleme hızı

## Network speed module
network-speed-heading = Ağ
network-speed-vpn = VPN
network-speed-vpn-off = Bağlı değil
network-speed-lan-ip = LAN IP
network-speed-vpn-ip = VPN IP

## Notification history sections
notifications-section-today = Bugün
notifications-section-yesterday = Dün
notifications-section-older = Daha eski

## DNS / VPN switcher module
dns-heading = DNS / VPN
dns-current-mode = Mod
dns-active-dns = Etkin DNS
dns-modes-title = Modlar
dns-providers-title = Sağlayıcılar
dns-mode-mullvad = Mullvad
dns-mode-blocky = Blocky
dns-mode-mixed = Mullvad + Blocky
dns-mode-default = Varsayılan (ISP)
dns-mode-unknown = Bilinmiyor
dns-action-toggle = Aç/Kapat
dns-action-repair = Onar

## UFW firewall module
ufw-heading = Güvenlik duvarı
ufw-active = Güvenlik duvarı: Aktif
ufw-inactive = Güvenlik duvarı: Pasif
ufw-unavailable = UFW yüklü değil
ufw-status = Durum
ufw-incoming = Gelen
ufw-outgoing = Giden
ufw-routed = Yönlendirilen
ufw-logging = Loglama
ufw-rule-count = Kurallar
ufw-rules-title = Son kurallar
ufw-action-toggle = Aç/Kapat
ufw-action-enable = Etkinleştir
ufw-action-disable = Devre dışı bırak
ufw-action-reload = Yeniden yükle

## Power module
power-heading = Güç
power-source = Kaynak
power-battery = Pil
power-profile = Profil
power-auto-lock = Otomatik profil
power-auto-lock-locked = Kilitli
power-auto-lock-unlocked = Açık
power-source-ac = AC
power-source-battery = Pil
power-source-unknown = Bilinmiyor
power-profiles-title = Profil
power-actions-title = İşlemler
power-profile-power-saver = Güç tasarrufu
power-profile-balanced = Dengeli
power-profile-performance = Performans
power-action-cycle = Döndür
power-action-lock-auto = Otomatik kilit
power-action-unlock = Kilidi aç
power-action-suspend = Askıya al
power-action-lock-screen = Ekranı kilitle

## Podman module
podman-heading = Podman
podman-unavailable = podman yüklü değil
podman-empty = Konteyner yok
podman-running = Çalışıyor
