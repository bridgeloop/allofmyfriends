// aiden@cmp.bz

mod api;
mod base64;

use std::{env::args, process::Command};
use api::{ApiError::*};

fn login() -> &'static str {
	eprintln!("log in, then re-run with argv[1] as your authorization code.");
	if Command::new("xdg-open").arg(api::LOGIN).spawn().is_err() {
		return "failed to launch browser";
	}
	return "not authenticated";
}
fn main() -> Result<(), &'static str> {
	let Some(auth) = args().nth(1) else {
		return Err(login());
	};

	let mut api = api::Api::new(auth).map_err(|err| match err {
		Eg(eg) if eg.code == "errors.com.epicgames.common.oauth.invalid_client" => {
			"invalid client - open an issue"
		}
		Eg(eg) if eg.code == "errors.com.epicgames.account.oauth.authorization_code_not_found" => {
			eprintln!("bad authorization code");
			return login();
		}
		other => {
			eprintln!("{other:?}");
			"unknown error"
		}
	})?;

	let resp = api.gql(api::FRIENDS_QUERY);
	println!("{resp:?}");
	
	return Ok(());
}
