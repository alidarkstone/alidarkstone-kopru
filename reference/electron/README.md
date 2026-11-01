# Köprü

İki Windows bilgisayarı tek masa gibi kullanmak için küçük, açık ve sade bir uygulama:

- **Fare ve klavye paylaşımı:** İmleci ekranın kenarından itince diğer bilgisayara geçer, geri iterseniz döner.
- **Pano paylaşımı:** Bir bilgisayarda kopyaladığınız metni diğerinde yapıştırırsınız.
- **Dosya gönderme:** Düğmeyle ya da sürükle-bırak ile; karşı taraf onaylamadan hiçbir şey kaydedilmez.

Hesap yok, bulut yok, telemetri yok. Cihazlar aynı yerel ağda doğrudan ve uçtan uca şifreli konuşur.
Her özellik **kapalı** başlar; siz açarsınız.

> Bu proje, VirusTotal'de işaretlenen "DuoDesk" kurulumunun incelenmesinden sonra **sıfırdan** yazıldı.
> DuoDesk'in kodu kopyalanmadı. Amaç, aynı temel işi yapan, her satırı okunabilir ve küçük bir uygulama.

## Kurulum (Windows 10/11)

Hazır `.exe` dağıtılmaz; her şeyi kendi bilgisayarınızda kaynaktan derlersiniz.

1. [Node.js LTS](https://nodejs.org) kurun.
2. Bu klasörde bir terminal (PowerShell) açın:
   ```powershell
   npm install
   npm run build-helper   # fare/klavye yardımcısını Windows'un kendi C# derleyicisiyle derler
   npm start
   ```
3. Aynısını ikinci bilgisayarda da yapın.
4. Windows Güvenlik Duvarı ilk açılışta izin sorarsa **Özel ağlar** için izin verin.

İsterseniz kurulum dosyası da üretebilirsiniz: `npm run dist` (çıktı `dist/` klasörüne gelir; imzasızdır).

Testler: `npm test` (Node.js ile çalışır, Electron gerektirmez).

## Kullanım

1. İki bilgisayarda Köprü'yü açın. "Ağdaki cihazlar" listesinde diğerini görürsünüz (görünmezse IP adresiyle bağlanın).
2. **Bağlan**'a basın. İki ekranda da 6 haneli bir kod çıkar. Kodlar **aynıysa** iki tarafta da "Aynı, eşleştir"e basın.
   Bu yalnızca ilk sefer gerekir; sonra cihazlar birbirini tanır ve otomatik bağlanır.
3. "Paylaşım" bölümünden istediğiniz özelliği açın:
   - **Fare ve klavye:** Her iki bilgisayarda da açın. "Diğer bilgisayar bu ekranın …" kısmından yerleşimi seçin
     (ör. A'da "sağında", B'de "solunda").
   - İmleç diğer bilgisayardayken **Scroll Lock** kontrolü hemen geri alır. Bağlantı koparsa kontrol kendiliğinden geri döner.
4. Dosya göndermek için "Dosya gönder"e basın veya dosyaları pencereye sürükleyin. Gelen dosyalar varsayılan olarak
   `İndirilenler\Köprü` klasörüne kaydedilir.

## Güvenlik tasarımı

| Konu | Nasıl |
|---|---|
| Kimlik | Her cihazın kalıcı bir Ed25519 anahtarı vardır (Windows DPAPI ile şifreli saklanır). |
| Eşleştirme | Geçici X25519 anahtar değişimi + iki tarafın imzası. 6 haneli kod bu oturumun transkriptinden türetilir; araya giren biri olursa kodlar farklı çıkar. İki taraf da onaylamadan hiçbir özellik çalışmaz. |
| Şifreleme | AES-256-GCM, her yön için ayrı anahtar ve sayaç. Değiştirilmiş, tekrar oynatılmış veya sırası bozulmuş paket bağlantıyı keser. |
| Dosyalar | Onaysız yazılmaz; ad temizlenir (klasör dışına yazılamaz); boyut ve SHA-256 tutmazsa silinir; var olan dosyanın üzerine yazılmaz. |
| Fare/klavye | Gelen olaylar yalnızca bilinen alanlarla ve sınırlar içinde uygulanır. Klavye kancası sadece imleç diğer bilgisayardayken kuruludur. |
| Arayüz | `contextIsolation`, `sandbox`, sıkı CSP; arayüz Node.js'e erişemez, başka sayfaya gidemez. |

### Köprü'nün yapmadıkları (DuoDesk'ten farklı olarak)

- Yönetici izni istemez, gizli Windows kullanıcısı açmaz, SSH anahtarı eklemez.
- Güvenlik duvarı kurallarını veya ağ profilini değiştirmez, sürücü indirmez/kurmaz.
- İnternete hiç bağlanmaz (güncelleme kontrolü, bağış sayfası, analitik yok).
- Hazır ikili dosya (`.exe`/`.dll`) içermez; tek yardımcı, `helper/windows/KopruInput.cs` dosyasından sizin bilgisayarınızda derlenir.

## Dosya yapısı

```
src/core/      framing.js (çerçeveleme), keys.js, channel.js (şifreli kanal), link.js (eşleştirme/oturum), discovery.js (UDP ile bulma)
src/features/  clipboard.js, files.js, input.js
src/ui/        arayüz (HTML/CSS/JS)
src/main.js    Electron ana süreç, src/preload.js arayüz köprüsü, src/store.js ayarlar
helper/windows KopruInput.cs (fare/klavye yardımcısı) ve build.cmd
test/          node:test testleri
```

Ağ portları: TCP 47910 (bağlantı), UDP 47911 (cihaz bulma).

## Bilinen sınırlar (v0.1)

- Fare/klavye paylaşımı yalnızca Windows'ta. macOS desteği sonraki adım.
- Pano yalnızca metin taşır (resim yok).
- Aynı anda tek eş cihaz.
- Yönetici olarak çalışan pencerelere (ör. Görev Yöneticisi) Windows kuralı gereği giriş gönderilemez. Ctrl+Alt+Del iletilemez.
- Çoklu monitörde kenar, tüm masaüstünün dış sınırıdır.
