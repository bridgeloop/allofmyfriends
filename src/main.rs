// aiden@cmp.bz

mod api;
use api::{Api, ApiError::*, Friends};

use dropfile::*;

use libc::{signal, SIGINT, SIGTERM};
use std::{env::args as Args, process::Command, io::{stdout, stdin, ErrorKind::WouldBlock, Write}, panic, time::Duration, collections::BTreeMap};
use serde_json::Value;
use tungstenite::{error::Error::Io, Message, stream::MaybeTlsStream::NativeTls};

use crate::api::{go_online, ApiError, GqlError};

// no real success condition here; should run forever
fn cont(api: &mut Api) -> Result<&'static str, &'static str> {
	println!("logged in as {}", api.token_response().display_name);
	fn e(err: ApiError<GqlError>) -> &'static str {
		return match err {
			Eg(f) if f.code == "errors.com.epicgames.common.authentication.token_verification_failed" => "refresh token expired",
			_ => "gql",
		};
	}
	let display_names: BTreeMap<String, String> = {
		let friends = api.gql::<Friends, _>(api::FRIENDS_QUERY).map_err(e)?;
		let friends = friends.0.into_iter().map(|f| {
			let display_name = f.display_name.unwrap_or_else(|| format!("[id:{}]", f.id));
			return (f.id, display_name);
		});

		friends.collect()
	};
	let (mut ws, _) = tungstenite::connect(
		format!("wss://connect.ol.epicgames.com/?auth-token={}", api.token_response().access_token)
	).map_err(|_| "failed to connect websocket")?;

	fn slice(msg: &[u8]) -> &[u8] {
		return &(msg[msg.windows(2).position(|sl| sl == b"\n\n").unwrap() + 2..msg.len() - 1]);
	}
	fn json(msg: Vec<u8>) -> Value {
		return serde_json::de::from_slice(slice(&(msg))).unwrap();
	}

	ws.write_message(Message::Text("SUBSCRIBE\n".to_owned())).unwrap();
	let Ok(Message::Binary(msg)) = ws.read_message() else {
		return Err("failed to read from ws");
	};
	
	let Value::String(connection_id) = json(msg)["connectionId"].take() else {
		return Err("failed to get connection id");
	};
	let status = api.gql::<Value, _>(go_online(&(connection_id))).map_err(e)?;
	if !status["data"]["PresenceV2"]["updateStatus"]["success"].as_bool().unwrap_or(false) {
		return Err("failed to set status to online");
	}

	let NativeTls(stream) = ws.get_mut() else {
		panic!("why the FUCK is it not a tls stream");
	};
	stream.get_mut().set_read_timeout(Some(Duration::from_secs(40))).unwrap();

	// return Ok on error after here, i guess?
	loop {
		match ws.read_message() {
			Ok(Message::Binary(msg)) => 'x: {
				let msg = json(msg);
				if msg["type"].as_str() != Some("presence.v1.UPDATE") {
					break 'x;
				}
				let payload = msg["payload"].as_object().expect("presence update must have payload");
				let id = payload["accountId"].as_str().expect("payload.accountId must be a string");
				let status = payload["status"].as_str().expect("payload.status must be a string");
				let display_name = display_names.get(id).expect("to-do: update display_names");
				#[cfg(debug_assertions)]
				let display_name = 
					format!("{}[id:{}]", display_name, id);
				match status {
					"online" | "offline" =>
						println!("{} is {}", display_name, status),
					_ => (),
				};
				#[cfg(debug_assertions)]
				println!("{msg:?}");
				break 'x;
			}
			Ok(_) => return Ok("unknown message type"),

			Err(Io(e)) if e.kind() == WouldBlock => (), // read timeout
			Err(_) => return Ok("failed to read from ws"),
		};
		if let Err(_) = ws.write_message(Message::Pong(vec![])) {
			return Ok("failed to write to ws");
		}
	}
}
fn cont_print(api: &mut Api) -> Result<(), &'static str> {
	let stop_reason = cont(api)?;
	println!("stopped because: {stop_reason}");
	return Ok(());
}

fn login(path: &str, run: bool) -> Result<(), &'static str> {
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

	let mut api = api::Api::new(auth.trim(), file).map_err(|err| match err {
		Eg(eg) if eg.code == "errors.com.epicgames.common.oauth.invalid_client" => {
			return "invalid client - open an issue";
		}
		Eg(eg) if eg.code == "errors.com.epicgames.account.oauth.authorization_code_not_found" => {
			return "bad authorization code";
		}
		other => {
			eprintln!("{other:?}");
			return "unknown error";
		}
	})?;

	if run {
		return cont_print(&mut(api));
	}
	return Ok(());
}

fn run(path: &str) -> Result<(), &'static str> {
	let file = DropFile::open(path, false)?;
	let mut api = api::Api::resume(file).map_err(|err| match err {
		_ => "session resume error",
	})?;
	return cont_print(&mut(api));
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

	let mut args = Args();
	let action = args.nth(1);
	return match action.as_deref() {
		Some("login") => {
			let path = args.next().ok_or("account path required")?;
			let run = args.next().as_deref() == Some("run");
			login(path.as_str(), run)
		}
		Some("run") => run(args.next().ok_or("account path required")?.as_str()),
		Some(path) if 'guess: {
			eprintln!("no action provided, trying `run`");
			let Err(err) = run(path) else {
				break 'guess true;
			};
			eprintln!("`run` failed: {err}");

			eprintln!("trying `login`");
			let run = args.next().as_deref() == Some("run");
			let Err(err) = login(path, run) else {
				break 'guess true;
			};
			eprintln!("`login` failed: {err}");
			
			break 'guess false;
		} => Ok(()),
		_ => {
			eprintln!("valid actions: login, run");
			Err("invalid action")
		}
	};
}
