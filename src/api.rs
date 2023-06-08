use {serde::{self, de::DeserializeOwned}, serde_json};

use std::{fmt::{Display, Debug}};

use dropfile::DropFile;
use reqwest::{blocking::{Client, ClientBuilder, Response, Body}, IntoUrl, header::{HeaderMap, HeaderValue}};

use {const_format::formatcp, base64};

// both of these values were taken from the macOS Epic Games Launcher
const CLIENT_ID: &'static str = "34a02cf8f4414e29b15921876da36f9a";
const CLIENT_SECRET: &'static str = "daafbccc737745039dffe53d94fc76cf";
const CLIENT_AUTH_STR: &'static str = formatcp!("{CLIENT_ID}:{CLIENT_SECRET}");
const CLIENT_AUTH_B64:
	[u8; base64::encode_sz::encoder_output_size_usize_panic(CLIENT_AUTH_STR.len())] =
	base64::encode_ct::array_from(
		CLIENT_AUTH_STR.as_bytes()
	);
const CLIENT_AUTH: &'static str = formatcp!(
	"basic {}",
	unsafe { core::mem::transmute::<&'static [u8], &'static str>(CLIENT_AUTH_B64.as_slice()) }
);

pub const LOGIN: &'static str = formatcp!(
	"https://www.epicgames.com/id/login?lang=en-US&redirectUrl=https%3A%2F%2Fwww.epicgames.com%2Fid%2Fapi%2Fredirect%3FclientId%3D{}%26responseType%3Dcode",
	CLIENT_ID
);

////////////////

#[derive(Debug, serde::Serialize)]
pub struct GqlOp<T: AsRef<str>> {
	query: T,
}
macro_rules! GqlOp {
	($str: expr) => {
		GqlOp { query: $str, }
	};
}

pub const FRIENDS_QUERY: GqlOp<&'static str> = GqlOp!(r#"query {
	Friends {
		summary(displayNames: true) {
			friends {
				account {
					id
					displayName
					displayNameType
					externalAuths {
						type
						externalAuthId
						externalAuthIdType
						externalDisplayName
						authIds {
							id
							type
						}
					}
				}
			}
		}
	}
}"#);
pub fn go_online(connection_id: &str) -> GqlOp<String> {
	return GqlOp {
		query: format!(r#"mutation {{
			PresenceV2 {{
				updateStatus(namespace: "_", connectionId: "{}", status: "online") {{
					success
				}}
			}}
		}}"#, connection_id)
	};
}

////////////////

pub trait ApiTokenExpired {
	fn token_expired(&self) -> bool;
}


#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct TokenResponse {
	pub access_token: String,
	expires_at: String,
	token_type: String,
	pub account_id: String,
	refresh_token: String,
	refresh_expires_at: String,

	#[serde(rename = "displayName")]
	pub display_name: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct TokenError {
	#[serde(rename(deserialize = "errorCode"))]
	pub code: String,
}
impl ApiTokenExpired for TokenError {
	fn token_expired(&self) -> bool {
		return self.code == "errors.com.epicgames.common.authentication.token_verification_failed" /* ? */;
	}
}

#[derive(Debug)]
pub struct FriendsAccount {
	pub display_name: Option<String>,
	pub id: String,
}
impl Display for FriendsAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    	return write!(f, "display name: {}\nid: {}\n", self.display_name.as_deref().unwrap_or("[none]"), self.id);
    }
}

pub struct Friends(pub Vec<FriendsAccount>);
impl<'de> Deserialize<'de> for Friends {
	fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		let mut value = serde_json::Value::deserialize(deserializer)?;
		let friends = value["data"]["Friends"]["summary"]["friends"]
			.as_array_mut()
			.ok_or(
				serde::de::Error::custom(
					"data.Friends.summary.friends is not an array"
				)
			)?;
		let mut vec = Vec::with_capacity(friends.len());
		for accounts in friends {
			let mut account = accounts["account"][0].take();
			let display_name = match account["displayName"].take() {
				serde_json::Value::String(s) => Some(s),
				_ => None,
			};
			let serde_json::Value::String(id) = account["id"].take() else {
				return Err(serde::de::Error::custom(
					"no account id?"
				));
			};
			vec.push(FriendsAccount {
				display_name, id,
			});
		}
		return Ok(Self(vec));
	}
}

#[derive(Debug)]
pub struct GqlError {
	pub code: String,
}
impl ApiTokenExpired for GqlError {
	fn token_expired(&self) -> bool {
		return self.code == "errors.com.epicgames.common.authentication.token_verification_failed";
	}
}
impl<'de> Deserialize<'de> for GqlError {
	fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		#[derive(Deserialize)]
		struct InnerStruct {
			#[serde(rename = "serviceResponse")]
			service_response: String,
		}
		#[derive(Deserialize)]
		struct FuckStruct {
			errors: [InnerStruct; 1],
		}
		let resp = FuckStruct::deserialize(deserializer)?;
		let tkn = serde_json::from_str::<TokenError>(&(resp.errors[0].service_response)).map_err(|_| serde::de::Error::missing_field("errorCode"))?;
		return Ok(Self { code: tkn.code });
	}
}


#[derive(Debug)]
pub enum ApiError<E> {
	Eg(E),
	In(&'static str),
	//Again,
}
use ApiError::*;
use serde::Deserialize;

////////////////

pub struct Api {
	cl: Client,
	tkn_resp: TokenResponse,

	eg1_cache: String,

	file: DropFile,
}
fn reqwest_cl<T>() -> Result<Client, ApiError<T>> {
	return ClientBuilder::new()
		.use_native_tls()
		.http2_prior_knowledge()
		.build()
		.map_err(|_| In("failed to build reqwest client"));
}
fn decode<T: DeserializeOwned, E: DeserializeOwned>(resp: Response) -> Result<T, ApiError<E>> {
	let resp = resp.text().map_err(|_| In("failed to read response"))?;
	#[cfg(debug_assertions)]
	eprintln!("{}", resp.trim());
	let error: serde_json::error::Result<E> = serde_json::from_str(&(resp));
	if let Ok(error) = error {
		return Err(ApiError::Eg(error));
	}
	return Ok(serde_json::from_str(&(resp)).map_err(|_| In("failed to decode response"))?);
}
fn eg1(tkn_resp: &TokenResponse) -> String {
	return format!("{} {}", tkn_resp.token_type, tkn_resp.access_token);
}
impl Api {
	fn call_internal<U: IntoUrl, B: Into<Body>, T: DeserializeOwned, E: DeserializeOwned + ApiTokenExpired>(
		&mut self,
		url: U,
		headers: Option<HeaderMap<HeaderValue>>,
		body: Option<B>
	) -> Result<T, ApiError<E>> {
		let mut req = if body.is_none() {
			self.cl.get(url.as_str())
		} else {
			self.cl.post(url.as_str())
		};
		if let Some(headers) = headers {
			req = req.headers(headers);
		}
		if let Some(body) = body {
			req = req.body(body);
		}
		let resp = req.send().map_err(|_| In("api call"))?;
		return decode(resp);
	}
	fn call<U: IntoUrl, B: Into<Body>, F: Fn(&Self) -> Option<(U, Option<HeaderMap<HeaderValue>>, Option<B>)>, T: DeserializeOwned, E: DeserializeOwned + ApiTokenExpired>(
		&mut self,
		gen_req: F,
	) -> Result<T, ApiError<E>> {
		let (url, headers, body) = gen_req(self).ok_or(In("gen_req failed"))?;
		let result = self.call_internal(url, headers, body);
		let Err(ApiError::Eg(err)) = &(result) else {
			return result;
		};
		if
			!err.token_expired() ||
			self.refresh().is_err()
		{
			return result;
		}
		
		let (url, headers, body) = gen_req(self).ok_or(In("gen_req failed"))?;
		return self.call_internal(url, headers, body);
	}
	fn refresh(&mut self) -> Result<(), ApiError<TokenError>> {
		let mut headers = HeaderMap::new();
		headers.insert("authorization", HeaderValue::from_static(CLIENT_AUTH));
		headers.insert("content-type", HeaderValue::from_static("application/x-www-form-urlencoded"));
		self.tkn_resp = self.call_internal(
			"https://account-public-service-prod03.ol.epicgames.com/account/api/oauth/token",
			Some(headers),
			Some(format!("grant_type=refresh_token&refresh_token={}&includePerms=false", self.token_response().refresh_token))
		)?;
		self.eg1_cache = eg1(&(self.tkn_resp));
		self.exp().map_err(|err| In(err))?;
		return Ok(());
	}

	pub fn new(auth: &str, file: DropFile) -> Result<Self, ApiError<TokenError>> {
		let cl = reqwest_cl()?;
		let exch = cl.post("https://account-public-service-prod03.ol.epicgames.com/account/api/oauth/token")
			.header("authorization", CLIENT_AUTH)
			.header("content-type", "application/x-www-form-urlencoded")
			.body(format!("grant_type=authorization_code&code={auth}&token_type=eg1"))
			.send()
			.map_err(|_| In("exch"))?;

		let tkn_resp: TokenResponse = decode(exch)?;
		let eg1 = eg1(&(tkn_resp));

		let mut s = Self {
			cl, tkn_resp,
			eg1_cache: eg1,
			file,
		};
		s.exp().map_err(|err| In(err))?;
		return Ok(s);
	}
	pub fn resume(mut file: DropFile) -> Result<Self, ApiError<TokenError>> {
		let cl = reqwest_cl()?;
		let tkn_resp: TokenResponse = serde_json::de::from_reader(&mut(file)).map_err(|_| In("failed to resume session"))?;
		let eg1 = eg1(&(tkn_resp));

		return Ok(Self {
			cl, tkn_resp,
			eg1_cache: eg1,
			file,
		});
	}

	fn exp(&mut self) -> Result<(), &'static str> {
		return serde_json::to_string(self.token_response())
			.map_err(|_| "failed to serialize token_response")
			.and_then(|tkn_resp|
				self.file.write_trunc(tkn_resp).map(|_| ())
			);
	}

	pub fn token_response(&self) -> &TokenResponse {
		return &(self.tkn_resp);
	}

	fn eg1(&self) -> &str {
		return self.eg1_cache.as_str();
	}

	pub fn gql<T: DeserializeOwned, S: AsRef<str> + serde::Serialize>(&mut self, op: GqlOp<S>) -> Result<T, ApiError<GqlError>> {
		// to everyone reading this:
		// did you know if your website has a user-agent whitelist, then you might [redacted]? [redacted] :-)
		const UA: &'static str = "\
			Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
			AppleWebKit/537.36 (KHTML, like Gecko) \
			EpicGamesLauncher/15.3.0-25516115+++Portal+Release-Live \
			UnrealEngine/4.27.0-25516115+++Portal+Release-Live \
			Chrome/90.0.4430.212 \
			Safari/537.36\
		";

		return self.call(|this: &Self| -> Option<(&str, Option<HeaderMap>, Option<String>)> {
			let mut headers = HeaderMap::new();
			headers.insert("authorization", HeaderValue::from_str(this.eg1()).unwrap());
			headers.insert("content-type", HeaderValue::from_static("application/json"));
			headers.insert("user-agent", HeaderValue::from_static(UA));

			return Some((
				"https://launcher.store.epicgames.com/graphql",
				Some(headers),
				Some(serde_json::to_string(&(op)).ok()?)
			));
		});
	}
}