# Serein ağ akışı

## Kısa cevap

Serein Discord'u yerelde taklit eden bir sunucu değildir. Discord masaüstü/web istemcisini
başlatması da gerekmez. Serein kendi HTTPS REST bağlantılarını ve Gateway WebSocket
bağlantısını kurar; Discord sunucuları bu istekleri yanıtlar ve gerçek zamanlı olayları
gönderir. UI, ağ yanıtı üretmez: kullanıcı eylemlerini tipli komutlara çevirir, gelen
olayları ekranda gösterir.

Bu, API isteklerini “Serein'in karşılaması” değil, istemcinin Discord'a istek göndermesi
anlamına gelir. Proje kendi mesajlaşma backend'ini çalıştırmaz; REST normalde doğrudan
Discord'a gider. Kullanıcının açıkça yetkilendirdiği bir API Proxy eklentisi istek rotasını
seçebilir, fakat Serein'in kendi backend'ine dönüşmez.

## İstekten ekrana

```text
egui UI
  -> client-core::Command (sınırlı kuyruk)
  -> apps/desktop bağlantı görevi
      -> discord-api: HTTPS REST
      -> discord-gateway: WebSocket ve sunucudan gelen olaylar
  <- discord-protocol: sınırlandırılmış JSON çözümleme
  <- model / client-core: durum güncelleme ve doğrulama
  <- UI olay kuyruğu: çizim

SQLite / session-cache: yerel, sınırlı geçmiş ve taslak saklama
```

### 1. UI ağ katmanına bağımlı değildir

`ui` kullanıcı eylemlerini `client-core::Command` değerlerine dönüştürür. Desktop
uygulamasındaki bağlantı görevi bu komutları alıp REST adaptörüne yollar; sonuçlar
`client-core::Event` olarak UI'ye geri döner. `crates/ui` HTTP isteği oluşturmaz ve oturum
sırrını taşımaz.

Kuyruklar, oturum nesilleri ve istek kimlikleri; eski bir kanal/hesap isteğinin geç
yanıtının yeni görünümü ezmesini önlemeye yardımcı olur. Ağ işi egui çizim döngüsünde
yapılmaz. Ses ve medya işleme de ayrı görevlerde tutulur.

### 2. REST tek bir adaptörde toplanır

`discord-api`, REST isteklerini `https://discord.com/api/v10` kökenine sabitler.
Çağıran UI'ye serbest URL vermez; her işlev kendi metot/yol/gövde biçimini oluşturur.
Oturum sırrı yalnızca adaptöre verilir, `Authorization` başlığı hassas olarak işaretlenir
ve istek/yanıt içeriği tanılama günlüğüne dökülmez.

Adaptörün önemli korumaları:

- TLS doğrulaması, yönlendirmeleri reddetme ve istek zaman aşımı;
- aynı anda en fazla dört REST isteği ve paylaşılan rate-limit beklemesi;
- yanıt gövdesi ile ayrıştırılan DTO'lar için boyut/sayı sınırları;
- belirsiz sonuç doğurabilecek yazma işlemlerini körlemesine tekrar etmeme;
- izin/veri/istek nesli denetimlerini yanıtı UI durumuna uygulamadan önce sürdürme.

Bunlar, her Discord davranışının desteklendiği anlamına gelmez. REST yollarının bir kısmı
resmi belgelenmiştir; bir kısmı normal hesaplarda kullanılan gayriresmî ve değişebilir
davranışlardır. Serein'in [uyumluluk matrisi](../discord-compatibility.md) bu ayrımı ve
canlı doğrulanmamış noktaları açıkça işaretler.

### 3. Gateway gerçek zamanlı akışı taşır

`discord-gateway` ayrı bir Tokio görevidir. Mevcut kaynakta Gateway adresi sabit
`wss://gateway.discord.gg` olarak başlatılır; bağlantı için REST'ten `/gateway` aranmaz.
İlk `READY` olayı hesap kimliğini ve oturum bilgisini sağlar; açılmış bir WebSocket tek
başına başarılı giriş sayılmaz. Hazır oturumda heartbeat/ACK takibi, olay sıra numarası
ve gerektiğinde Resume/yeniden bağlanma mantığı bulunur.

Gateway JSON'u önce `discord-protocol` DTO'larına çözülür. Büyük başlangıç anlık
görüntüsü (READY) ile sıradan olaylar farklı boyut limitlerine sahiptir. Bozuk veya
kapasiteyi aşan paketler sessizce uygulanmak yerine bağlantı hatasına dönüşür. Olaylar
`client-core` tarafından seçili hesap/kanal/istek bağlamıyla eşleştirilir.

**Uyumluluk sınırı:** Serein'in normal kullanıcı hesabı için kullandığı Identify ve bazı
Gateway olayları resmi bot API'sinin destek garantisi kapsamında değildir. Reponun
belgeleri bunları gayriresmî olarak sınıflandırır; derleme veya sentetik testler canlı
Discord uyumluluğunu kanıtlamaz.

### 4. JSON doğrudan UI durumuna yazılmaz

`discord-protocol` ham JSON'u tanımlı DTO'lara ve daha sonra `model` tiplerine dönüştürür.
Eksik/null alanlar ve kısmi güncellemeler açık tiplerle temsil edilir. Boyut, dizi
uzunluğu, metin ve ilişki kontrolleri uygulanır. Böylece ağ yanıtı UI'de kontrolsüz
nesne grafiğine veya sınırsız belleğe dönüşmez.

### 5. Yerel saklama uzak API değildir

`session-cache` etkin konuşmanın sınırlı RAM penceresini yönetir. `local-store` SQLite
üzerinde hesaplara ayrılmış geçmiş/taslak gibi verileri saklar. Bunlar Discord'dan gelen
verinin yerel kopyalarıdır; Discord adına istek karşılamaz ve uzak durumu taklit etmez.
Kaydedilmiş kimlik bilgileri için OS credential store tercih edilir. İlgili sınırlar
[saklama politikasında](../storage-policy.md) açıklanır.

Giriş webview'i varsa yalnızca kullanıcı tarafından başlatılan geçici kimlik doğrulama
akışıdır; uygulama başka bir Discord kurulumunun profilini/oturumunu taramaz. Ağ
oturumunun geri kalanı yine Serein'in yerel REST ve Gateway taşımasında çalışır.

## Yeni uygulama için çıkarımlar

Serein'in mimari ayrımı iyi bir mühendislik örneğidir: UI, alan modeli, protokol
çözümleme, REST, Gateway ve saklama birbirinden ayrıdır. Fakat Serein'in normal-hesap
protokol ayrıntılarını başka uygulamada yeniden kullanmak, Discord tarafından destek
veya izin olduğu anlamına gelmez. Kendi uygulamanızı resmi Bot API/OAuth2 kapsamlarında
tasarlayın; önerilen ilk adımlar [Rust istemci rehberindedir](build-a-rust-client.md).

## Kodda izlenecek yerler

- `apps/desktop/src/connection.rs`: görevler, kuyruklar, REST/Gateway bağlama noktaları
- `crates/discord-api/src/lib.rs`: sabit kökenli REST taşıması ve sınırlar
- `crates/discord-gateway/src/lib.rs`: Gateway bağlantısı, heartbeat, READY ve Resume
- `crates/discord-protocol/src/lib.rs`: JSON çözümleme ve tel DTO'ları
- `crates/client-core/src/lib.rs`: UI ile taşıma arasındaki komut/olay tipleri
- `docs/architecture.md`: RAM/disk/kuyruk limitleri ve genel bileşen sorumlulukları
