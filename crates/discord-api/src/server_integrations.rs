use crate::{DiscordApi, Failure};
use discord_protocol::server_integrations as wire;
use model::{
	Id,
	server_integrations::{Action, Snapshot},
};
use reqwest::Method;
use serde_json::json;
impl DiscordApi {
	pub(super) async fn copy_webhook_url(
		&self,
		guild: Id,
		webhook: Id,
		channel: Id,
	) -> Result<model::server_integrations::WebhookUrl, Failure> {
		let bytes = self
			.request_limited(
				Method::GET,
				&format!("/webhooks/{webhook}"),
				None,
				64 * 1024,
			)
			.await?;
		let bytes = zeroize::Zeroizing::new(bytes);
		wire::webhook_url(&bytes, guild, webhook, channel).map_err(|_| {
			Failure::ProtocolAt("Webhook URL was unavailable or did not match this channel")
		})
	}

	async fn integration_snapshot(
		&self,
		guild: Id,
		integrations: bool,
		webhooks: bool,
		channel: Option<Id>,
	) -> Result<Snapshot, Failure> {
		let mut page = Snapshot {
			guild,
			channel,
			integrations: None,
			webhooks: None,
		};
		if integrations {
			let bytes = self
				.request_limited(
					Method::GET,
					&format!("/guilds/{guild}/integrations"),
					None,
					wire::MAX_WIRE,
				)
				.await?;
			page.integrations = wire::integrations(&bytes, guild)
				.map_err(|_| {
					Failure::ProtocolAt(
						"Integrations exceeded safe bounds or used an unsupported response",
					)
				})?
				.integrations;
		}
		if webhooks {
			let path = channel.map_or_else(
				|| format!("/guilds/{guild}/webhooks"),
				|channel| format!("/channels/{channel}/webhooks"),
			);
			let bytes = self
				.request_limited(Method::GET, &path, None, wire::MAX_WIRE)
				.await?;
			page.webhooks = wire::webhooks(&bytes, guild)
				.map_err(|_| {
					Failure::ProtocolAt(
						"Webhooks exceeded safe bounds or used an unsupported response",
					)
				})?
				.webhooks;
		}
		if !page.valid() {
			return Err(Failure::Capacity);
		}
		Ok(page)
	}
	async fn integration_channel(&self, guild: Id, channel: Id) -> Result<(), Failure> {
		let bytes = self
			.request_limited(
				Method::GET,
				&format!("/channels/{channel}"),
				None,
				64 * 1024,
			)
			.await?;
		wire::channel_scope(&bytes, guild, channel).map_err(|_| Failure::Forbidden)
	}
	pub(super) async fn server_integration_action(
		&self,
		guild: Id,
		action: &Action,
	) -> Result<Snapshot, Failure> {
		if guild.0 == 0 || !action.valid() {
			return Err(Failure::Protocol);
		}
		let mut saved = None;
		let scope = action.scope();
		match action {
			Action::CopyWebhookUrl { .. } => return Err(Failure::Protocol),
			Action::Load {
				integrations,
				webhooks,
				..
			} => {
				return self
					.integration_snapshot(guild, *integrations, *webhooks, scope)
					.await;
			}
			Action::CreateWebhook { channel, name, .. } => {
				self.integration_channel(guild, *channel).await?;
				let bytes = self
					.request_limited(
						Method::POST,
						&format!("/channels/{channel}/webhooks"),
						Some(json!({"name":name})),
						64 * 1024,
					)
					.await
					.map_err(write_failure)?;
				let value = wire::webhook(&bytes, guild).map_err(|_| Failure::Ambiguous)?;
				if value.kind != 1
					|| value.channel != Some(*channel)
					|| value.name.as_ref() != Some(name)
				{
					return Err(Failure::Ambiguous);
				}
				saved = Some(value.id);
			}
			Action::EditWebhook {
				webhook,
				channel,
				name,
				..
			} => {
				let latest = self.integration_snapshot(guild, false, true, scope).await?;
				let Some(current) = latest
					.webhooks
					.as_ref()
					.and_then(|items| items.iter().find(|item| item.id == *webhook))
				else {
					return Err(Failure::Forbidden);
				};
				if current.kind != 1 {
					return Err(Failure::Forbidden);
				}
				self.integration_channel(guild, *channel).await?;
				let bytes = self
					.request_limited(
						Method::PATCH,
						&format!("/webhooks/{webhook}"),
						Some(json!({"name":name,"channel_id":channel.to_string()})),
						64 * 1024,
					)
					.await
					.map_err(write_failure)?;
				let value = wire::webhook(&bytes, guild).map_err(|_| Failure::Ambiguous)?;
				if value.id != *webhook
					|| value.kind != 1
					|| value.channel != Some(*channel)
					|| value.name.as_ref() != Some(name)
				{
					return Err(Failure::Ambiguous);
				}
				saved = Some(value.id);
			}
			Action::DeleteWebhook { webhook, .. } => {
				let latest = self.integration_snapshot(guild, false, true, scope).await?;
				let Some(current) = latest
					.webhooks
					.as_ref()
					.and_then(|items| items.iter().find(|item| item.id == *webhook))
				else {
					return Ok(latest);
				};
				if !(1..=3).contains(&current.kind) {
					return Err(Failure::Forbidden);
				}
				let bytes = self
					.request_limited(Method::DELETE, &format!("/webhooks/{webhook}"), None, 4096)
					.await
					.map_err(write_failure)?;
				if !bytes.is_empty() {
					return Err(Failure::Ambiguous);
				}
			}
			Action::DeleteIntegration { integration } => {
				let latest = self.integration_snapshot(guild, true, false, None).await?;
				if !latest
					.integrations
					.as_ref()
					.is_some_and(|items| items.iter().any(|item| item.id == *integration))
				{
					return Ok(latest);
				}
				let bytes = self
					.request_limited(
						Method::DELETE,
						&format!("/guilds/{guild}/integrations/{integration}"),
						None,
						4096,
					)
					.await
					.map_err(write_failure)?;
				if !bytes.is_empty() {
					return Err(Failure::Ambiguous);
				}
			}
		}
		let integration_write = matches!(action, Action::DeleteIntegration { .. });
		let page = self
			.integration_snapshot(guild, integration_write, !integration_write, scope)
			.await
			.map_err(reconcile_failure)?;
		let failed = match action {
			Action::EditWebhook {
				webhook, channel, ..
			} if scope.is_some_and(|scope| scope != *channel) => page
				.webhooks
				.as_ref()
				.is_none_or(|items| items.iter().any(|item| item.id == *webhook)),
			Action::CreateWebhook { channel, name, .. }
			| Action::EditWebhook { channel, name, .. } => !page.webhooks.as_ref().is_some_and(|items| {
				items.iter().any(|item| {
					Some(item.id) == saved
						&& item.channel == Some(*channel)
						&& item.name.as_ref() == Some(name)
						&& item.kind == 1
				})
			}),
			Action::DeleteWebhook { webhook, .. } => page
				.webhooks
				.as_ref()
				.is_some_and(|items| items.iter().any(|item| item.id == *webhook)),
			Action::DeleteIntegration { integration } => page
				.integrations
				.as_ref()
				.is_some_and(|items| items.iter().any(|item| item.id == *integration)),
			Action::Load { .. } | Action::CopyWebhookUrl { .. } => false,
		};
		if failed {
			return Err(Failure::Ambiguous);
		}
		Ok(page)
	}
}
fn write_failure(failure: Failure) -> Failure {
	if failure == Failure::Capacity {
		Failure::Ambiguous
	} else {
		failure
	}
}
fn reconcile_failure(failure: Failure) -> Failure {
	if failure.ends_session() {
		failure
	} else {
		Failure::Ambiguous
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::SessionSecret;
	use std::{sync::Arc, time::Duration};
	use tokio::{
		io::{AsyncReadExt, AsyncWriteExt},
		net::TcpListener,
	};
	#[tokio::test]
	async fn webhook_url_copy_uses_authenticated_get_and_rejects_wrong_scope() {
		tokio::time::timeout(Duration::from_secs(10), async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let mut api = DiscordApi::new(Arc::new(SessionSecret::from_owner_input("SYNTHETIC_ACCOUNT_TOKEN".into()).unwrap())).unwrap();
            api.base = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                for channel in ["4", "99"] {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let mut request = Vec::new();
                    loop {
                        let mut chunk = [0; 2048]; let n = stream.read(&mut chunk).await.unwrap(); assert!(n > 0);
                        request.extend_from_slice(&chunk[..n]); assert!(request.len() < 8192);
                        if request.windows(4).any(|part| part == b"\r\n\r\n") { break; }
                    }
                    let request = std::str::from_utf8(&request).unwrap();
                    assert!(request.starts_with("GET /webhooks/3 HTTP/1.1\r\n"));
                    assert!(request.lines().any(|line| line.eq_ignore_ascii_case("authorization: SYNTHETIC_ACCOUNT_TOKEN")));
                    let body = json!({"id":"3","guild_id":"2","channel_id":channel,"type":1,"token":"SYNTHETIC_WEBHOOK_TOKEN"}).to_string();
                    stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
                }
            });
            let url = api.copy_webhook_url(Id(2), Id(3), Id(4)).await.unwrap();
            assert_eq!(url.expose(), "https://discord.com/api/webhooks/3/SYNTHETIC_WEBHOOK_TOKEN");
            assert!(api.copy_webhook_url(Id(2), Id(3), Id(4)).await.is_err());
            server.await.unwrap();
        }).await.unwrap();
	}

	#[tokio::test]
	async fn integrations_http_permission_scopes_mutation_reconciliation_and_no_retry() {
		for scope in [None, Some(Id(4))] {
			tokio::time::timeout(Duration::from_secs(10),async {
			let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();
			let mut api=DiscordApi::new(Arc::new(SessionSecret::from_owner_input("SYNTHETIC_INTEGRATIONS_TOKEN".into()).unwrap())).unwrap();
			api.base=format!("http://{}",listener.local_addr().unwrap());
			let channel=json!({"id":"4","guild_id":"2","type":0});
			let hook=json!({"id":"3","guild_id":"2","channel_id":"4","type":1,"name":"Builds","token":"SYNTHETIC_EXECUTION_SECRET"});
			let edited=json!({"id":"3","guild_id":"2","channel_id":"4","type":1,"name":"Releases"});
			let integration=json!({"id":"5","name":"Example","type":"discord","enabled":true});
			let hook_path = if scope.is_some() {"/channels/4/webhooks"} else {"/guilds/2/webhooks"};
			let server=tokio::spawn(async move {
				for (method,path,status,response,body) in [
					("GET","/guilds/2/integrations",200,json!([integration.clone()]),None),
					("GET",hook_path,200,json!([hook.clone()]),None),
					("GET","/channels/4",200,channel.clone(),None),
					("POST","/channels/4/webhooks",200,hook.clone(),Some(json!({"name":"Builds"}))),
					("GET",hook_path,200,json!([hook.clone()]),None),
					("GET",hook_path,200,json!([hook]),None),
					("GET","/channels/4",200,channel,None),
					("PATCH","/webhooks/3",200,edited.clone(),Some(json!({"name":"Releases","channel_id":"4"}))),
					("GET",hook_path,200,json!([edited.clone()]),None),
					("GET",hook_path,200,json!([edited.clone()]),None),
					("DELETE","/webhooks/3",204,serde_json::Value::Null,None),
					("GET",hook_path,200,json!([]),None),
					("GET","/guilds/2/integrations",200,json!([integration]),None),
					("DELETE","/guilds/2/integrations/5",204,serde_json::Value::Null,None),
					("GET","/guilds/2/integrations",200,json!([]),None),
					("GET",hook_path,200,json!([edited.clone()]),None),
					("DELETE","/webhooks/3",500,json!({}),None),
					("GET","/channels/4/webhooks",200,json!([edited]),None),
					("GET","/channels/6",200,json!({"id":"6","guild_id":"2","type":0}),None),
					("PATCH","/webhooks/3",200,json!({"id":"3","guild_id":"2","channel_id":"6","type":1,"name":"Moved"}),Some(json!({"name":"Moved","channel_id":"6"}))),
					("GET","/channels/4/webhooks",200,json!([]),None),
					("GET","/channels/4/webhooks",200,json!([{"id":"3","guild_id":"2","channel_id":"6","type":1,"name":"Moved"}]),None),
				] {
					let (mut stream,_)=listener.accept().await.unwrap(); let mut bytes=Vec::new();
					let end=loop { let mut chunk=[0;4096]; let n=stream.read(&mut chunk).await.unwrap(); assert!(n>0); bytes.extend_from_slice(&chunk[..n]); assert!(bytes.len()<=16384); if let Some(end)=bytes.windows(4).position(|part|part==b"\r\n\r\n") { break end+4; } };
					let headers=std::str::from_utf8(&bytes[..end]).unwrap(); assert!(headers.starts_with(&format!("{method} {path} HTTP/1.1\r\n")));
					let length=headers.lines().find_map(|line| { let (name,value)=line.split_once(':')?; name.eq_ignore_ascii_case("content-length").then(||value.trim().parse::<usize>().unwrap()) }).unwrap_or(0); assert!(length<=4096);
					while bytes.len()<end+length { let mut chunk=[0;4096];let n=stream.read(&mut chunk).await.unwrap();assert!(n>0);bytes.extend_from_slice(&chunk[..n]); }
					if let Some(body)=body { assert_eq!(serde_json::from_slice::<serde_json::Value>(&bytes[end..end+length]).unwrap(),body); } else { assert_eq!(length,0); }
					let response=if status==204 {String::new()} else {response.to_string()}; stream.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).as_bytes()).await.unwrap();
				}
				assert!(tokio::time::timeout(Duration::from_millis(80),listener.accept()).await.is_err());
			});
			let page=api.server_integration_action(Id(2),&Action::Load { channel: None,integrations:true,webhooks:false}).await.unwrap(); assert!(page.webhooks.is_none());
			let page=api.server_integration_action(Id(2),&Action::Load {channel:scope,integrations:false,webhooks:true}).await.unwrap(); assert_eq!(page.channel,scope);
			let page=api.server_integration_action(Id(2),&Action::CreateWebhook { scope,channel:Id(4),name:"Builds".into()}).await.unwrap(); assert_eq!(page.webhooks.unwrap()[0].id,Id(3));
			let page=api.server_integration_action(Id(2),&Action::EditWebhook { scope,webhook:Id(3),channel:Id(4),name:"Releases".into()}).await.unwrap(); assert_eq!(page.webhooks.unwrap()[0].name.as_deref(),Some("Releases"));
			assert!(api.server_integration_action(Id(2),&Action::DeleteWebhook { scope,webhook:Id(3)}).await.unwrap().webhooks.unwrap().is_empty());
			assert!(api.server_integration_action(Id(2),&Action::DeleteIntegration {integration:Id(5)}).await.unwrap().integrations.unwrap().is_empty());
			assert!(matches!(api.server_integration_action(Id(2),&Action::DeleteWebhook { scope,webhook:Id(3)}).await,Err(Failure::Ambiguous)));
			let page=api.server_integration_action(Id(2),&Action::EditWebhook {scope:Some(Id(4)),webhook:Id(3),channel:Id(6),name:"Moved".into()}).await.unwrap(); assert_eq!(page.channel,Some(Id(4))); assert!(page.webhooks.unwrap().is_empty());
			assert!(api.server_integration_action(Id(2),&Action::Load {channel:Some(Id(4)),integrations:false,webhooks:true}).await.is_err());
			server.await.unwrap();
		}).await.unwrap();
		}
	}
}
