// aiden@cmp.bz

mod api;
use dropfile::*;

use std::{env::{self, Args}, process::Command, io::{stdin, stdout, Write}, panic};
use api::ApiError::*;
use libc::{signal, SIGINT, SIGTERM};

fn cont(mut api: api::Api) -> Result<(), &'static str> {
	return Ok(());
}

fn login(mut args: Args) -> Result<(), &'static str> {
	let path =
		args.next().ok_or("account path required")?;
	let mut file = DropFile::open(&(path), true)?;

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

	let api = api::Api::new(auth.trim()).map_err(|err| match err {
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
	api.exp(file).map_err(|_| "failed to write file")?;

	if args.next().as_deref() == Some("run") {
		return cont(api);
	}
	return Ok(());
}

fn run(mut args: Args) -> Result<(), &'static str> {
	let path =
		args.next().ok_or("account path required")?;
	let mut file = DropFile::open(&(path), false)?;
	let api = api::Api::resume(&mut(file)).map_err(|err| match err {
		_ => "session resume error",
	})?;
	api.exp(file).map_err(|_| "failed to write file")?;
	println!("successfully resumed");
	return cont(api);
}

fn main() -> Result<(), &'static str> {
	unsafe {
		fn term() -> ! {
			panic::resume_unwind(Box::new(()));
		}
		let term = term as *const () as usize;
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
