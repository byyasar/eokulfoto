# Webcam Capture 🦀 (e-Devlet Biyometrik: 133x171)

Webcam'den görüntü yakalayıp **e-Devlet biyometrik fotoğraf standartlarına** dönüştüren yüksek performanslı, ultra hafif yerel **Rust** uygulaması:

| Standart / Ölçü | Değer |
|---|---|
| **Ebat** | 35 × 45 mm (Baskı DPI bilgisi dosyaya gömülür) |
| **Çözünürlük** | 133 × 171 px (varsayılan, `.env` üzerinden ayarlanabilir) |
| **Dosya Boyutu** | 20 – 150 KB (e-Devlet kısıtlamasına göre otomatik optimize edilir) |
| **İkili Boyut** | Yalnızca ~2.2 MB (bağımsız tek dosya, Python gerektirmez) |

Ayarlar `.env` dosyasından okunur ve arayüz üzerinden değiştirilebilir.

---

## 🚀 Çalıştırma

### 1. macOS Uygulaması Olarak Çalıştırma (Önerilen)
Doğrudan üretilen macOS uygulamasını açabilir veya `Applications` (Uygulamalar) klasörüne taşıyabilirsiniz:
```bash
open "dist/Webcam Capture.app"
```
Veya DMG yükleyicisini açıp tek sürüklemeyle yükleyebilirsiniz:
```bash
open "dist/WebcamCapture-Installer.dmg"
```

> **macOS .app Paketi ve DMG'yi Yeniden Derlemek İçin:**
> ```bash
> ./scripts/build_mac_app.sh
> ```
> Bu script otomatik olarak:
> - Apple standartlarında çoklu çözünürlüklü `AppIcon.icns` ikonunu oluşturur.
> - Kamera izin tanımlarını (`NSCameraUsageDescription`, `entitlements.plist`) paketler.
> - `dist/Webcam Capture.app` paketini ve `dist/WebcamCapture-Installer.dmg` kalıbını üretir ve ad-hoc kod imzalamasını tamamlar.

### 2. Geliştirme Modunda Çalıştırma (Cargo)
```bash
cargo run --release
```
*(veya doğrudan derlenmiş dosyayı çalıştırın: `./target/release/webcam_capture`)*

Sistemin yerel WebKit penceresinde çalışan modern macOS arayüzü:
- **Kamera Seçimi:** Sistemdeki kameraları otomatik listeler ve dinamik geçiş sağlar.
- **Canlı Kamera Önizlemesi:** Biyometrik hizalama kılavuzları ile (sol).
- **Hedef Önizleme:** 133×171 piksellik nihai çıktı (sağ).
- **🖥️ İkinci Monitör / Ayrı Pencere:** Stüdyo çekimleri için hedef görüntüyü bağımsız bir ekrana taşıyan ikinci pencere desteği (tam ekran, canlı kılavuzlar, uzaktan SPACE ile çekim).
- **Özel Dosya Adı:** Kaydedilecek fotoğrafa ana ekrandan özel isim verebilme.
- **📸 Anlık Çekim:** **SPACE** veya dosya adı kutusunda **Enter** kısayolu.
- **Klasörü Aç:** Kaydedilen fotoğrafların bulunduğu klasörü tek tıkla açar.
- **Ayarlar Paneli:** Kamera seçimi, netlik (HD/Full HD), çözünürlük, format (`png`/`jpg`), JPEG kalitesi, e-Devlet boyut kısıtı (20–150 KB).

### 2. Komut Satırı (CLI Modu)
Arayüz açmadan doğrudan terminalden tek kare çekim yapmak için:
```bash
cargo run -- --once --name "ornek_vesikalik"
```

### 3. Yalnızca Sunucu Modu (Harici / Tablet Ekranı İçin)
```bash
cargo run -- --server-only
```

---

## 🛠️ Derleme

Optimize edilmiş tek parça ikili dosya üretmek için:
```bash
cargo build --release
```
Çıktı konumu: `target/release/webcam_capture` (~2.2 MB)

---

## 📁 Proje Yapısı

- `src/` — Rust kaynak kodları:
  - `main.rs`: Giriş noktası, yerel HTTP sunucusu ve Tao/Wry pencere yönetimi.
  - `camera.rs`: AVFoundation/MediaFoundation kamera sürücüsü, kılavuzlar ve biyometrik kırpma.
  - `edevlet.rs`: e-Devlet 20–150 KB sınırlandırması, DPI gömme ve resim kaydetme.
  - `app_state.rs`: Thread-safe durum yönetimi ve RPC köprüsü.
  - `config.rs`: `.env` yapılandırma yöneticisi.
- `webui/` — Modern Web tabanlı arayüz (HTML, CSS, JS).
- `Cargo.toml` — Rust paket ve derleme ayarları.
