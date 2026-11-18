# Resmi API ile Rust uygulamasına başlama

## Önce hangi ürünü yaptığını seç

Discord'un resmi API'si, normal bir kullanıcı hesabının tam masaüstü istemcisini
üçüncü taraf Rust uygulamasında yeniden oluşturma API'si değildir.

- **Bot veya bot yönetim aracı:** Discord Developer Portal'da oluşturulan bot kimliği ve
  belgelenmiş Bot API/Gateway akışları.
- **Kullanıcı adına OAuth bağlantısı:** resmi OAuth2 akışı ve yalnızca kullanıcının verdiği
  kapsamlar. OAuth2, Discord'un tüm birinci taraf istemci özelliklerine erişim sağlamaz.
- **Normal kullanıcı token'ı ile “self-bot”:** bunu kullanmayın. Kendi hesabınızın token'ını
  bir bot token'ı gibi göndermek, özel istemci protokolünü taklit etmek veya başka bir
  uygulamadan token çıkarmak desteklenen bir yol değildir ve hesap riski taşır.

Bu nedenle aşağıdaki örnek, yalnızca kendi bot kimliğiyle kimlik bilgisi okuyan küçük bir
**resmi Bot API** başlangıç denemesidir. Bir Serein özelliğini normal kullanıcı hesabıyla
çoğaltmaz.

## 1. Minimal proje

Rust projesi oluşturup `tokio`, `reqwest` (rustls ve `stream` özellikleriyle), `serde` ve
`serde_json` ekleyin.
Token'ı kaynak koda, uygulama paketine, log'a veya hata mesajına koymayın. Yerel
geliştirmede `DISCORD_BOT_TOKEN` ortam değişkeninden okunabilir; dağıtılan masaüstü
ikilisinin içine bot sırrı gömülmemelidir.

Bu örnek HTTPS üzerinden belgelenmiş `/users/@me` okuması yapar, yönlendirmeyi reddeder
ve JSON gövdesini 64 KiB ile sınırlar:

```rust
use reqwest::header::{AUTHORIZATION, HeaderValue};
use serde::Deserialize;
use std::{io, time::Duration};

const MAX_BODY: usize = 64 * 1024;

#[derive(Deserialize)]
struct BotIdentity {
    id: String,
    username: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let token = std::env::var("DISCORD_BOT_TOKEN")?;
    if token.is_empty() || token.len() > 4096 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid token length").into());
    }

    let mut authorization = HeaderValue::from_str(&format!("Bot {token}"))?;
    authorization.set_sensitive(true);

    let client = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()?;

    let mut response = client
        .get("https://discord.com/api/v10/users/@me")
        .header(AUTHORIZATION, authorization)
        .send()
        .await?
        .error_for_status()?;

    if response
        .content_length()
        .is_some_and(|length| length > MAX_BODY as u64)
    {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "response too large").into());
    }

    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len().saturating_add(chunk.len()) > MAX_BODY {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "response too large").into());
        }
        body.extend_from_slice(&chunk);
    }

    let bot: BotIdentity = serde_json::from_slice(&body)?;
    println!("Connected as {} ({})", bot.username, bot.id);
    Ok(())
}
```

Örnek sadece bağlantıyı ve serileştirmeyi gösterir. Gerçek uygulamada token'ı
`Zeroizing` gibi bir sarmalayıcıyla yönetmeyi, hatalarda hassas değerleri asla yazmamayı
ve masaüstü ürününde bot sırrı dağıtmamayı değerlendirin. Rate limit yanıtlarına
`Retry-After`/Discord başlıklarına göre uyun; tüm yazma isteklerini otomatik tekrar
etmeyin. Güncel sözleşme için daima [resmi API referansını](https://discord.com/developers/docs/intro)
kullanın.

## 2. Projeyi katmanlara ayır

Önerilen küçük başlangıç düzeni:

```text
src/
  app/          UI ve kullanıcı akışları
  model/        uygulama alan tipleri
  protocol/     serde DTO'ları ve doğrulama
  api/          REST istemcisi ve tipli operasyonlar
  gateway/      bot kullanıyorsa WebSocket oturumu
  state/        olay indirgeme ve oturum nesli
  storage/      yerel ayarlar ve kullanıcı verisi
```

UI'den doğrudan ağ çağırmak yerine sınırlı bir komut kuyruğu kullanın. API/Gateway
görevleri typed event'ler üretir; UI thread yalnızca state'i indirger ve çizer. Her yanıtı
hesap/kanal/istek nesliyle eşleştirin ki gezinme değiştikten sonra dönen eski yanıt yeni
ekranı ezmesin. Ağ, dosya, görsel çözme ve ses işini render/audio callback'lerinden uzak
tutun.

## 3. REST için sağlam varsayılanlar

1. API kökenini sabitleyin; dış girdiden tam URL kabul etmeyin.
2. HTTPS doğrulaması, makul bağlantı/toplam zaman aşımı ve yönlendirme politikası belirleyin.
3. Eşzamanlı istek, yanıt baytı, JSON dizisi ve saklanan nesne sayısına üst sınır koyun.
4. `429` durumunu ve `Retry-After`/rate-limit başlıklarını uygulayın; limitleri atlatmaya
   çalışmayın.
5. Güvenli/idempotent okumalarla belirsiz yazma sonuçlarını ayırın. Sunucu yanıtı
   kayboldu diye mesaj/işlem yazmasını körlemesine tekrarlamayın.
6. Sadece gereken DTO alanlarını ayrıştırın; bozuk veya beklenmedik yanıtı açık hata olarak
   sunun. Başarı gibi görünen boş varsayılanlar üretmeyin.
7. Yetkiyi sunucu belirler. UI'deki rol/izin kontrolü güvenlik sınırı değildir.

## 4. Gateway gerekiyorsa

Bot Gateway bağlantısı için Discord'un güncel resmi belgelerini izleyin: Hello/heartbeat,
Identify, istenen intents, Dispatch sıra numaraları, `READY`, `INVALID_SESSION`, Resume
ve close kodları. Gerekli olmayan privileged intent'leri istemeyin. Yeniden bağlanma
beklemelerini sınırlayın; her bağlantı hatasında yeni Identify döngüsü başlatmayın.
Event kuyrukları dolarsa mesajları sessizce kaybetmek yerine oturumu kontrollü durdurun
veya yeniden senkronize edin.

Normal kullanıcı oturumları için Serein'in kullandığı özel/kararsız Gateway davranışını
bu bot akışına kopyalamayın. Kodun varlığı veya sentetik fixture'ların geçmesi, Discord'un
normal hesap istemcilerine destek verdiğini göstermez.

## 5. Önce çevrimdışı doğrula

- UI, reducer ve hata akışlarını sentetik verilerle çalıştırın.
- REST için loopback/mock sunucu kullanın; zaman aşımı, yanlış JSON, büyük gövde, 401/403,
  429 ve yönlendirme durumlarını sınayın.
- Gateway çözümleyicisini sentetik frame'lerle sınayın; canlı hesaba bağlanmayı test
  varsayılanı yapmayın.
- Yerel cache, kuyruk ve payload'ları hem bayt hem öğe sayısına göre sınırlayın.
- Gerçek hesapla test gerekiyorsa yalnızca uygulamanın sahibi tarafından kontrol edilen
  açıkça yetkilendirilmiş geliştirici oturumunda, küçük ve geri alınabilir adımlarla yapın.
  Şifre, token, MFA kodu veya özel mesajı kaynak koda/log'a/CI'ya koymayın.

## Serein'den alınabilecek parçalar

Serein'in **katman ayrımı, sınırlandırılmış kuyrukları, typed DTO/event akışı ve sentetik
test yaklaşımı** incelenebilir. Normal hesapla kimlik doğrulama, özel endpoint'ler ve
istemci kimliği taklidi ise genel amaçlı bir starter kit değildir.

- [Kod akışı](how-serein-works.md)
- [Discord uyumluluk sınıfları](../discord-compatibility.md)
- [Mimari ve kapasite sınırları](../architecture.md)
- [Kimlik bilgisi saklama politikası](../storage-policy.md)
- [Resmi Gateway belgeleri](https://discord.com/developers/docs/events/gateway)
- [Resmi OAuth2 belgeleri](https://discord.com/developers/docs/topics/oauth2)
