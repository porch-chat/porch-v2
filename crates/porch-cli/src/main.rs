use porch_protocol::{
    AuthResponse, CreateInviteRequest, CreatedInvite, Invite, LoginRequest, RegisterRequest,
    Session, User,
};
use reqwest::{
    Method,
    blocking::{Client, Response},
};
use serde::Serialize;
use std::{env, error::Error, time::Duration};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

struct Cli {
    client: Client,
    base: String,
}

impl Cli {
    fn request<T: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: Option<&T>,
        auth: bool,
    ) -> Result<Response> {
        let mut request = self.client.request(method, format!("{}{path}", self.base));
        if auth {
            request = request.bearer_auth(env::var("PORCH_TOKEN")?);
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send()?;
        if !response.status().is_success() {
            // Do not echo request credentials or raw server text.
            return Err(format!("request failed with HTTP {}", response.status()).into());
        }
        Ok(response)
    }

    fn read<T: serde::de::DeserializeOwned + Serialize>(&self, path: &str) -> Result<()> {
        let response: T = self.request::<()>(Method::GET, path, None, true)?.json()?;
        print_json(&response)
    }

    fn delete(&self, path: &str) -> Result<()> {
        self.request::<()>(Method::DELETE, path, None, true)?;
        println!("ok");
        Ok(())
    }
}

fn print_json<T: Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(command) = args.first().map(String::as_str) else {
        return Err("usage: porch-cli register|login|me|sessions|logout|logout-all|revoke-session ID|invite-create USES SECONDS|invites|invite-revoke ID; credentials from PORCH_USERNAME/PORCH_PASSWORD/PORCH_INVITE/PORCH_TOKEN".into());
    };
    let base = env::var("PORCH_URL").unwrap_or_else(|_| "http://127.0.0.1:38147".into());
    let url = reqwest::Url::parse(&base)?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
        || !matches!(url.scheme(), "http" | "https")
    {
        return Err(
            "PORCH_URL must be an HTTP(S) origin without credentials, path or query".into(),
        );
    }
    if url.scheme() == "http"
        && !matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
    {
        return Err("HTTPS required except for loopback development".into());
    }
    let cli = Cli {
        client: Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()?,
        base: base.trim_end_matches('/').into(),
    };
    let expected = match command {
        "invite-create" => 3,
        "revoke-session" | "invite-revoke" => 2,
        _ => 1,
    };
    if args.len() != expected {
        return Err("incorrect number of arguments".into());
    }
    match command {
        "register" => {
            let body = RegisterRequest {
                username: env::var("PORCH_USERNAME")?,
                password: env::var("PORCH_PASSWORD")?,
                invite_code: env::var("PORCH_INVITE")?,
            };
            let result: AuthResponse = cli
                .request(Method::POST, "/auth/register", Some(&body), false)?
                .json()?;
            print_json(&result)?;
        }
        "login" => {
            let body = LoginRequest {
                username: env::var("PORCH_USERNAME")?,
                password: env::var("PORCH_PASSWORD")?,
            };
            let result: AuthResponse = cli
                .request(Method::POST, "/auth/login", Some(&body), false)?
                .json()?;
            print_json(&result)?;
        }
        "me" => cli.read::<User>("/auth/me")?,
        "sessions" => cli.read::<Vec<Session>>("/auth/sessions")?,
        "logout" => {
            cli.request::<()>(Method::POST, "/auth/logout", None, true)?;
            println!("ok");
        }
        "logout-all" => cli.delete("/auth/sessions")?,
        "revoke-session" => {
            let id: i64 = args[1].parse()?;
            cli.delete(&format!("/auth/sessions/{id}"))?;
        }
        "invite-create" => {
            let body = CreateInviteRequest {
                max_uses: args[1].parse()?,
                expires_in_seconds: args[2].parse()?,
            };
            let result: CreatedInvite = cli
                .request(Method::POST, "/invites", Some(&body), true)?
                .json()?;
            print_json(&result)?;
        }
        "invites" => cli.read::<Vec<Invite>>("/invites")?,
        "invite-revoke" => {
            let id: i64 = args[1].parse()?;
            cli.delete(&format!("/invites/{id}"))?;
        }
        _ => return Err("unknown command".into()),
    }
    Ok(())
}
