# Rust ile Discord istemcisi: Serein nasıl çalışıyor?

Bu klasör, Serein'in mevcut ağ/mimari akışını ve yeni bir Rust uygulamasına nasıl
başlanabileceğini açıklar. Serein kodunun teknik bir incelemesidir; Discord tarafından
desteklenen bir normal-hesap API'si veya kararlı bir protokol tarifi değildir.

## Rehber

- [Serein'in istek ve olay akışı](how-serein-works.md)
- [Resmi API ile Rust uygulamasına başlama](build-a-rust-client.md)

## Önemli ayrım

Discord'un belgelenmiş API'si uygulama/bot kimlikleri ve OAuth2 kapsamları içindir.
Normal kullanıcı hesabını üçüncü taraf bir masaüstü istemcisinde otomatikleştirmek
resmi API'nin desteklediği bir kullanım değildir; Discord'un şartları ve hesap güvenliği
açısından risk taşır. Serein bu nedenle resmi/garantili bir istemci değil, mevcut
Discord hesaplarına bağlanan **gayriresmî** bir projedir. Bu rehber özel normal-hesap
rotalarını veya doğrulama/rate-limit aşma yöntemlerini öğretmez.

## Kaynaklar

- [Resmi Discord API belgeleri](https://discord.com/developers/docs/intro)
- [Resmi Gateway belgeleri](https://discord.com/developers/docs/events/gateway)
- [OAuth2 belgeleri](https://discord.com/developers/docs/topics/oauth2)
- Serein'in [mimari özeti](../architecture.md), [kimlik doğrulama notları](../authentication.md),
  [uyumluluk matrisi](../discord-compatibility.md), [saklama politikası](../storage-policy.md)
  ve [tehdit modeli](../threat-model.md)
