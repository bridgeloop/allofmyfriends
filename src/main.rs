// aiden@cmp.bz

mod api;
use api::{Api, ApiError::{self, *}, FriendsError};

use dropfile::*;

use libc::{signal, SIGINT, SIGTERM};
use std::{env::{self, Args}, process::Command, io::{stdin, stdout, Write}, panic};
use tungstenite;

fn cont(api: &mut Api) -> Result<(), &'static str> {
	let friends: serde_json::Value = api.gql(api::FRIENDS_QUERY).map_err(|_: ApiError<FriendsError>| "gql")?;
	println!("{:?}", friends);
	let (mut ws, _) = tungstenite::connect(
		format!("wss://connect.ol.epicgames.com/?auth-token={}", api.token_response().eg1())
	).map_err(|_| "failed to connect websocket")?;
	loop {
		let msg = ws.read_message().expect("read_message");
		println!("{msg}");
	}
}

fn login(mut args: Args) -> Result<(), &'static str> {
	let path =
		args.next().ok_or("account path required")?;
	let file = DropFile::open(path, true)?;

	if Command::new("xdg-open").arg(api::LOGIN).spawn().is_err() {
		println!("{}", api::LOGIN);
	}
	let mut lines = stdin().lines();
	let auth = loop {
		print!("authorization code: ");
		drop(stdout().flush()); // doesn't matter if this fails
		let Some(Ok(ln)) = lines.next() else {
			return Err("failed to read from stdin");
		};

		let t = ln.trim();
		if t.len() == 32 && t.chars().all(|c| !c.is_ascii_uppercase() && c.is_ascii_hexdigit()) {
			break ln;
		}
	};
	println!("proceeding!");

	let mut api = api::Api::new(auth.trim(), file).map_err(|err| match err {
		Eg(eg) if eg.code == "errors.com.epicgames.common.oauth.invalid_client" => {
			return "invalid client - open an issue";
		}
		Eg(eg) if eg.code == "errors.com.epicgames.account.oauth.authorization_code_not_found" => {
			return "bad authorization code";
		}
		other => {
			eprintln!("{other:?}");
			return "unknown error"
		}
	})?;

	return if args.next().as_deref() == Some("run") {
		cont(&mut(api))
	} else {
		Ok(())
	};
}

fn run(mut args: Args) -> Result<(), &'static str> {
	let path =
		args.next().ok_or("account path required")?;
	let file = DropFile::open(path, false)?;
	let mut api = api::Api::resume(file).map_err(|err| match err {
		_ => "session resume error",
	})?;

	println!("successfully resumed");
	return cont(&mut(api));
}

fn main() -> Result<(), &'static str> {
	fn term() -> ! {
		panic::resume_unwind(Box::new(()));
	}
	let term = term as *const () as usize;
	unsafe {
		signal(SIGINT, term);
		signal(SIGTERM, term);
	}

	let mut args = env::args();
	let action = args.nth(1);
	return match action.as_deref() {
		Some("login") => login(args),
		Some("run") => run(args),
		_ => {
			eprintln!("valid actions: login, run");
			return Err("invalid action");
		}
	};
}
