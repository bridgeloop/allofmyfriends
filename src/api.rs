use {serde::{self, de::DeserializeOwned}, serde_json};
use std::io::{Read, Seek};

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
pub struct GqlOp {
	query: &'static str,
}
macro_rules! GqlOp {
	($str: expr) => {
		GqlOp { query: $str, }
	};
}

pub const FRIENDS_QUERY: GqlOp = GqlOp!(r#"query {
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

////////////////

pub trait ApiTokenExpired {
	fn token_expired(&self) -> bool;
}


#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct TokenResponse {
	access_token: String,
	expires_at: String,
	token_type: String,
	account_id: String,
	refresh_token: String,
	refresh_expires_at: String,

	#[serde(rename = "displayName")]
	display_name: String,
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

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct FriendsResponse {
}

#[derive(Debug)]
pub struct FriendsError {
	pub code: String,
}
impl ApiTokenExpired for FriendsError {
	fn token_expired(&self) -> bool {
		return self.code == "errors.com.epicgames.common.authentication.token_verification_failed";
	}
}
impl<'de> Deserialize<'de> for FriendsError {
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
}
use ApiError::In;
use serde::Deserialize;

////////////////

pub struct Api {
	cl: Client,
	tkn_resp: TokenResponse,

	eg1_cache: String,
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
		url: &U,
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
	fn call<U: IntoUrl, B: Into<Body> + Clone, T: DeserializeOwned, E: DeserializeOwned + ApiTokenExpired>(
		&mut self,
		url: U,
		headers: Option<HeaderMap<HeaderValue>>,
		body: Option<B>
	) -> Result<T, ApiError<E>> {
		let headers_clone = headers.clone();
		let body_clone = body.clone();

		let r = self.call_internal(&(url), headers, body);
		let Err(ApiError::Eg(err)) = &(r) else {
			return r;
		};
		if !err.token_expired() {
			return r;
		}
		if self.refresh().is_err() {
			return r;
		}
		return self.call_internal(&(url), headers_clone, body_clone);
	}
	fn refresh(&mut self) -> Result<(), ApiError<TokenError>> {
		let mut headers = HeaderMap::new();
		headers.insert("authorization", HeaderValue::from_static(CLIENT_AUTH));
		headers.insert("content-type", HeaderValue::from_static("application/x-www-form-urlencoded"));
		self.tkn_resp = self.call_internal(
			&("https://account-public-service-prod03.ol.epicgames.com/account/api/oauth/token"),
			Some(headers),
			Some(format!("grant_type=refresh_token&refresh_token={}&includePerms=false", self.token_response().refresh_token))
		)?;
		self.eg1_cache = eg1(&(self.tkn_resp));
		return Ok(());
	}

	pub fn new(auth: &str) -> Result<Self, ApiError<TokenError>> {
		let cl = reqwest_cl()?;
		let exch = cl.post("https://account-public-service-prod03.ol.epicgames.com/account/api/oauth/token")
			.header("authorization", CLIENT_AUTH)
			.header("content-type", "application/x-www-form-urlencoded")
			.body(format!("grant_type=authorization_code&code={auth}&token_type=eg1"))
			.send()
			.map_err(|_| In("exch"))?;

		let tkn_resp: TokenResponse = decode(exch)?;
		let eg1 = eg1(&(tkn_resp));

		return Ok(Self {
			cl, tkn_resp,
			eg1_cache: eg1,
		});
	}
	pub fn resume<T: Read>(ctx: &mut T) -> Result<Self, ApiError<TokenError>> {
		let cl = reqwest_cl()?;
		let tkn_resp: TokenResponse = serde_json::de::from_reader(ctx).map_err(|_| In("failed to resume session"))?;
		let eg1 = eg1(&(tkn_resp));

		return Ok(Self {
			cl, tkn_resp,
			eg1_cache: eg1,
		});
	}

	pub fn exp(&self, mut to: DropFile) -> Result<(), ()> {
		to.rewind().map_err(|_| ())?;
		serde_json::to_writer(&mut(to), self.token_response()).map_err(|_| ())?;
		to.trunc_to_cursor().map_err(|_| ())?;
		return Ok(());
	}

	pub fn token_response(&self) -> &TokenResponse {
		return &(self.tkn_resp);
	}

	fn eg1(&self) -> &str {
		return self.eg1_cache.as_str();
	}

	pub fn gql<T: DeserializeOwned, E: DeserializeOwned + ApiTokenExpired>(&mut self, op: GqlOp) -> Result<T, ApiError<E>> {
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

		let mut headers = HeaderMap::new();
		headers.insert("authorization", HeaderValue::from_str(self.eg1()).unwrap());
		headers.insert("content-type", HeaderValue::from_static("application/json"));
		headers.insert("user-agent", HeaderValue::from_static(UA));
		let body = serde_json::to_string(&(op)).map_err(|_| In("to_string"))?;
		return self.call("https://launcher.store.epicgames.com/graphql", Some(headers), Some(body));
	}
}