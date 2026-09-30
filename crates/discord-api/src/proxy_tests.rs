use super::*;
use proxy::ApiProxy;
use tokio::{
	io::{AsyncReadExt, AsyncWriteExt},
	net::{TcpListener, TcpStream},
	sync::watch,
	time::timeout,
};

async fn headers(stream: &mut TcpStream) -> String {
	let mut bytes = Vec::new();
	while !bytes.ends_with(b"\r\n\r\n") {
		assert!(bytes.len() < 4096);
		bytes.push(stream.read_u8().await.unwrap());
	}
	String::from_utf8(bytes).unwrap()
}
fn api(receiver: watch::Receiver<Option<ApiProxy>>) -> DiscordApi {
	DiscordApi::with_proxy(
		Arc::new(SessionSecret::from_owner_input("SYNTHETIC_PROXY_TOKEN".into()).unwrap()),
		receiver,
	)
	.unwrap()
}

#[tokio::test]
async fn waits_for_configuration_then_routes_future_requests_and_rejects_invalid_changes() {
	let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let (route, receive) = watch::channel(None);
	let mut api = api(receive);
	api.base = "http://synthetic-api.invalid/api/v10".into();
	assert!(
		timeout(Duration::from_millis(25), api.rest_client())
			.await
			.is_err()
	);
	route.send_replace(Some(ApiProxy::Url {
		url: format!("http://{}", proxy.local_addr().unwrap()),
	}));
	let server = tokio::spawn(async move {
		let (mut stream, _) = proxy.accept().await.unwrap();
		let request = headers(&mut stream).await;
		assert!(
			request.starts_with("GET http://synthetic-api.invalid/api/v10/synthetic HTTP/1.1\r\n")
		);
		assert!(
			request
				.to_ascii_lowercase()
				.contains("authorization: synthetic_proxy_token\r\n")
		);
		stream
			.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
			.await
			.unwrap();
	});
	assert_eq!(
		api.request(Method::GET, "/synthetic", None).await.unwrap(),
		b"{}"
	);
	server.await.unwrap();
	route.send_replace(Some(ApiProxy::Url {
		url: "http://user:secret@proxy.invalid".into(),
	}));
	assert!(api.rest_client().await.is_err());
	// Rejected settings cannot reuse the old connection; disabling restores direct routing.
	route.send_replace(Some(ApiProxy::Direct));
	let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
	api.base = format!("http://{}", origin.local_addr().unwrap());
	let server = tokio::spawn(async move {
		let (mut stream, _) = origin.accept().await.unwrap();
		assert!(
			headers(&mut stream)
				.await
				.starts_with("GET /synthetic HTTP/1.1\r\n")
		);
		stream
			.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
			.await
			.unwrap();
	});
	assert_eq!(
		api.request(Method::GET, "/synthetic", None).await.unwrap(),
		b"{}"
	);
	server.await.unwrap();
}

#[tokio::test]
async fn rejected_connect_does_not_expose_discord_token_or_fall_back_direct() {
	let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let (_, receive) = watch::channel(Some(ApiProxy::Url {
		url: format!("http://{}", proxy.local_addr().unwrap()),
	}));
	let mut api = api(receive);
	api.base = format!("https://{}", origin.local_addr().unwrap());
	let server = tokio::spawn(async move {
		let (mut stream, _) = proxy.accept().await.unwrap();
		let request = headers(&mut stream).await;
		assert!(request.starts_with("CONNECT 127.0.0.1:"));
		assert!(!request.to_ascii_lowercase().contains("authorization:"));
		assert!(!request.contains("SYNTHETIC_PROXY_TOKEN"));
		stream.write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
	});
	assert_eq!(
		timeout(
			Duration::from_secs(3),
			api.request(Method::GET, "/synthetic", None)
		)
		.await
		.unwrap(),
		Err(Failure::Network)
	);
	server.await.unwrap();
	assert!(
		timeout(Duration::from_millis(50), origin.accept())
			.await
			.is_err()
	);
}
