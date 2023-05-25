use {serde::{self, de::DeserializeOwned}, serde_json};
use reqwest::blocking::{Client, ClientBuilder, Response};

use {const_format::formatcp, base64};

// both of these values were taken from the macOS Epic Games Launcher
const CLIENT_ID: &'static str = "34a02cf8f4414e29b15921876da36f9a";
const CLIENT_SECRET: &'static str = "daafbccc737745039dffe53d94fc76cf";
const CLIENT_AUTH_STR: &'static str = formatcp!("{CLIENT_ID}:{CLIENT_SECRET}");
const CLIENT_AUTH:
	[u8; base64::encode_sz::encoder_output_size_usize_panic(CLIENT_AUTH_STR.len())] = 
	base64::encode_ct::array_from(
		CLIENT_AUTH_STR.as_bytes()
	);

pub const LOGIN: &'static str = formatcp!(
	"https://www.epicgames.com/id/login?lang=en-US&redirectUrl=https%3A%2F%2Fwww.epicgames.com%2Fid%2Fapi%2Fredirect%3FclientId%3D{}%26responseType%3Dcode",
	CLIENT_ID
);

fn status<T: DeserializeOwned, E: DeserializeOwned>(resp: Response) -> Result<T, ApiError<E>> {
	if resp.status() != 200 {
		let error_text = resp.text().map_err(|_| In("failed to read response"))?;
		let error: E = serde_json::from_str(&(error_text)).map_err(|_err| {
			//eprintln!("{_err:?} {error_text:?}");
			return In("failed to decode error");
		})?;
		return Err(ApiError::Eg(error));
	}
	return Ok(resp.json().map_err(|_| In("failed to decode response"))?);
}

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


#[derive(Debug)]
pub enum ApiError<E> {
	Eg(E),
	In(&'static str),
}
use ApiError::In;

////////////////

pub struct Api {
	cl: Client,
	tkn_resp: TokenResponse,

	eg1_cache: String,
}
impl Api {
	pub fn new(auth: &str) -> Result<Self, ApiError<TokenError>> {
		let cl = ClientBuilder::new()
			.use_native_tls()
			.http2_prior_knowledge()
			.build()
			.map_err(|_| In("failed to build reqwest client"))?;
		let exch = cl.post("https://account-public-service-prod03.ol.epicgames.com/account/api/oauth/token")
			.header("authorization", CLIENT_ID)
			.header("content-type", "application/x-www-form-urlencoded")
			.body(format!("grant_type=authorization_code&code={auth}&token_type=eg1"))
			.send()
			.map_err(|_| In("exch"))?;

		let tkn_resp: TokenResponse = status(exch)?;
		let eg1 = format!("{} {}", tkn_resp.token_type, tkn_resp.access_token);

		return Ok(Self {
			cl, tkn_resp,
			eg1_cache: eg1,
		});
	}

	pub fn token_response(&self) -> &TokenResponse {
		return &(self.tkn_resp);
	}

	fn eg1(&self) -> &str {
		return self.eg1_cache.as_str();
	}

	pub fn gql<T: serde::de::DeserializeOwned>(&mut self, op: GqlOp) -> Result<T, ApiError<()>> {
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
		
		let resp = self.cl.post("https://launcher.store.epicgames.com/graphql")
			.header("authorization", self.eg1())
			.header("content-type", "application/json")
			.header("user-agent", UA)
			.body(serde_json::to_string(&(op)).map_err(|_| In("to_string"))?)
			.send()
			.map_err(|err| { eprintln!("{err:?}"); In("gql") })?;
		return status(resp);
	}
}