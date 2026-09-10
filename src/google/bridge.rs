//! Per-process Rclone token bridge. Only Boreal owns Google's refresh token.
//! Temporary configs contain a short-lived local capability, never that refresh token.
use super::{GoogleError, auth};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub struct Bridge {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    path: PathBuf,
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
        let _ = fs::remove_file(&self.path);
    }
}
impl Bridge {
    pub fn attach(command: &mut Command, config: &Path) -> Result<Option<Self>, GoogleError> {
        let conf = config.parent().ok_or("Invalid Rclone configuration path")?;
        if !conf.join("google-account.json").exists() {
            return Ok(None);
        }
        let args = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let writes = args
            .iter()
            .any(|a| a.starts_with("my-drive-rw:") || a.starts_with("my-drive-rw,"));
        if writes && !auth::setup_at(conf)?.migration_access {
            return Err(
                "Enable migration write access in Settings → Google connection first.".into(),
            );
        }
        let scope = if writes {
            auth::DRIVE_WRITE
        } else {
            auth::DRIVE_READ
        };
        // Verify before starting Rclone. Do not fall back to a different legacy account.
        auth::access_at(conf, scope)?;
        let account = auth::account_key_at(conf)?;
        let conf = conf.to_path_buf();
        let mut random = [0u8; 32];
        getrandom::fill(&mut random)
            .map_err(|_| "Unable to secure the local Google token bridge")?;
        let secret = URL_SAFE_NO_PAD.encode(random);
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let endpoint = format!("http://127.0.0.1:{}/token", listener.local_addr()?.port());
        let path = conf.join(format!(
            ".google-rclone-{:x}.conf",
            Sha256::digest(secret.as_bytes())
        ));
        let token=serde_json::json!({"access_token":"pending","token_type":"Bearer","refresh_token":secret,"expiry":"1970-01-01T00:00:00Z"}).to_string();
        let mut config_text = String::new();
        for remote in ["my-drive-ro", "my-drive-rw"] {
            config_text.push_str(&format!("[{remote}]\ntype = drive\nclient_id = boreal-local\nclient_secret =\nscope = {}\ntoken_url = {endpoint}\ntoken = {token}\n\n",if writes {"drive"} else {"drive.readonly"}));
        }
        auth::private_write(&path, config_text.as_bytes())?;
        // Replace prior --config in place: a command must have exactly one config.
        let mut new_args = Vec::new();
        let mut skip = false;
        for arg in args {
            if skip {
                skip = false;
                continue;
            }
            if arg == "--config" {
                skip = true;
                continue;
            }
            if arg.starts_with("--config=") {
                continue;
            }
            new_args.push(arg);
        }
        let program = command.get_program().to_owned();
        let mut replacement = Command::new(program);
        replacement.args(new_args).arg("--config").arg(&path);
        // All callers attach before setting stdio. Preserve explicit environment.
        for (key, value) in command.get_envs() {
            if let Some(value) = value {
                replacement.env(key, value);
            } else {
                replacement.env_remove(key);
            }
        }
        if let Some(dir) = command.get_current_dir() {
            replacement.current_dir(dir);
        }
        let bypass = format!(
            "{},127.0.0.1,localhost",
            std::env::var("NO_PROXY").unwrap_or_default()
        );
        replacement
            .env("NO_PROXY", &bypass)
            .env("no_proxy", &bypass);
        // A shell's old Rclone OAuth overrides must not select a different account.
        for (name, _) in std::env::vars_os() {
            let upper = name.to_string_lossy().to_ascii_uppercase();
            if upper.starts_with("RCLONE_CONFIG_MY_DRIVE_RO_")
                || upper.starts_with("RCLONE_CONFIG_MY_DRIVE_RW_")
                || upper.starts_with("RCLONE_CONFIG_MY-DRIVE-RO_")
                || upper.starts_with("RCLONE_CONFIG_MY-DRIVE-RW_")
                || [
                    "RCLONE_DRIVE_TOKEN",
                    "RCLONE_DRIVE_CLIENT_ID",
                    "RCLONE_DRIVE_CLIENT_SECRET",
                    "RCLONE_DRIVE_TOKEN_URL",
                    "RCLONE_DRIVE_AUTH_URL",
                    "RCLONE_DRIVE_SERVICE_ACCOUNT_FILE",
                    "RCLONE_DRIVE_SERVICE_ACCOUNT_CREDENTIALS",
                    "RCLONE_DRIVE_CLIENT_CREDENTIALS",
                ]
                .contains(&upper.as_str())
            {
                replacement.env_remove(name);
            }
        }
        *command = replacement;
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                        let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                        let response = if authorized(&mut stream, &secret) {
                            if auth::account_key_at(&conf).is_ok_and(|key| key == account) {
                                auth::access_at(&conf,scope).map(|(access,expires)|serde_json::json!({"access_token":access,"token_type":"Bearer","expires_in":expires,"refresh_token":secret}).to_string())
                            } else {
                                Err("Google account changed during this operation".into())
                            }
                        } else {
                            Err("Unauthorized token request".into())
                        };
                        let (status,body)=match response {Ok(body)=>("200 OK",body),Err(_)=>("400 Bad Request",r#"{"error":"invalid_grant","error_description":"Google connection needs attention in Boreal Settings"}"#.into())};
                        let _ = write!(
                            stream,
                            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
                            body.len()
                        );
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(25))
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Some(Self {
            stop,
            thread: Some(handle),
            path,
        }))
    }
}
fn authorized(stream: &mut TcpStream, secret: &str) -> bool {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut bytes = Vec::new();
    let mut byte = [0u8; 1];
    while bytes.len() < 8192 && Instant::now() < deadline {
        if stream.read(&mut byte).unwrap_or(0) != 1 {
            return false;
        }
        bytes.push(byte[0]);
        if bytes.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let Ok(headers) = std::str::from_utf8(&bytes) else {
        return false;
    };
    if !headers.starts_with("POST /token HTTP/1.1\r\n") || !headers.ends_with("\r\n\r\n") {
        return false;
    }
    let lengths = headers
        .lines()
        .filter_map(|l| l.split_once(':'))
        .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .collect::<Vec<_>>();
    if lengths.len() != 1
        || headers
            .lines()
            .any(|l| l.to_ascii_lowercase().starts_with("transfer-encoding:"))
    {
        return false;
    }
    let Ok(length) = lengths[0].1.trim().parse::<usize>() else {
        return false;
    };
    if length > 8192 {
        return false;
    }
    let mut body = vec![0; length];
    if stream.read_exact(&mut body).is_err() {
        return false;
    }
    let pairs = reqwest::Url::parse(&format!(
        "http://localhost/?{}",
        String::from_utf8_lossy(&body)
    ))
    .ok()
    .map(|u| {
        u.query_pairs()
            .map(|(a, b)| (a.into_owned(), b.into_owned()))
            .collect::<Vec<_>>()
    })
    .unwrap_or_default();
    pairs.iter().filter(|(k, _)| k == "refresh_token").count() == 1
        && pairs
            .iter()
            .any(|(k, v)| k == "refresh_token" && v == secret)
        && pairs
            .iter()
            .any(|(k, v)| k == "grant_type" && v == "refresh_token")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_rclone_renews_through_broker_without_google_network() {
        let Some(executable) = std::env::var_os("BOREAL_TEST_RCLONE") else {
            return;
        };
        let runtime = auth::tests::fixture();
        let conf = auth::conf(&runtime).unwrap();
        let config = conf.join("rclone.conf");
        fs::write(&config, "[extra]\ntype = local\n").unwrap();
        let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
        proxy.set_nonblocking(true).unwrap();
        let proxy_url = format!("http://{}", proxy.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let proxy_thread = thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                if let Ok((mut stream, _)) = proxy.accept() {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(1)))
                        .unwrap();
                    let mut buf = [0; 8192];
                    let _ = stream.read(&mut buf);
                    let _=stream.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                } else {
                    thread::sleep(Duration::from_millis(10));
                }
            }
        });
        let mut command = Command::new(executable);
        command
            .args([
                "backend",
                "drives",
                "my-drive-ro:",
                "--json",
                "--retries",
                "1",
                "--low-level-retries",
                "1",
                "--contimeout",
                "2s",
                "--timeout",
                "2s",
                "--config",
            ])
            .arg(&config);
        let bridge = Bridge::attach(&mut command, &config).unwrap().unwrap();
        command
            .env("HTTPS_PROXY", &proxy_url)
            .env("https_proxy", &proxy_url)
            .env("HTTP_PROXY", &proxy_url)
            .env("http_proxy", &proxy_url)
            .env("NO_PROXY", "127.0.0.1,localhost")
            .env("no_proxy", "127.0.0.1,localhost");
        let output = command.output().unwrap();
        stop.store(true, Ordering::Relaxed);
        proxy_thread.join().unwrap();
        assert!(
            !output.status.success(),
            "the fixture blocks actual Google API access"
        );
        let text = fs::read_to_string(&bridge.path).unwrap();
        assert!(
            text.contains("synthetic-access"),
            "Rclone must refresh its expired local token before contacting the blocked Drive API"
        );
        assert!(!text.contains("synthetic-private-refresh"));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("synthetic-access"));
        assert_eq!(
            fs::read_to_string(&config).unwrap(),
            "[extra]\ntype = local\n"
        );
        drop(bridge);
        fs::remove_dir_all(&runtime.boreal_home).unwrap();
    }
    #[test]
    fn broker_is_private_account_bound_and_preserves_legacy_config() {
        let runtime = auth::tests::fixture();
        let conf = auth::conf(&runtime).unwrap();
        let path = conf.join("rclone.conf");
        fs::write(
            &path,
            "[extra]\ntype = local\n[my-drive-ro]\ntoken = old-private-token\n",
        )
        .unwrap();
        let before = fs::read(&path).unwrap();
        let mut cmd = Command::new("rclone");
        cmd.args(["backend", "drives", "my-drive-ro:", "--config"])
            .arg(&path);
        let bridge = Bridge::attach(&mut cmd, &path).unwrap().unwrap();
        let temp = fs::read_to_string(&bridge.path).unwrap();
        assert!(!temp.contains("synthetic-private-refresh"));
        assert!(!temp.contains("old-private-token"));
        let endpoint = temp
            .lines()
            .find_map(|l| l.strip_prefix("token_url = "))
            .unwrap();
        let token: serde_json::Value = serde_json::from_str(
            temp.lines()
                .find_map(|l| l.strip_prefix("token = "))
                .unwrap(),
        )
        .unwrap();
        let secret = token["refresh_token"].as_str().unwrap();
        assert!(!format!("{cmd:?}").contains(secret));
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .build()
            .unwrap();
        assert_eq!(
            client
                .post(endpoint)
                .form(&[("grant_type", "refresh_token"), ("refresh_token", "wrong")])
                .send()
                .unwrap()
                .status()
                .as_u16(),
            400
        );
        for _ in 0..2 {
            let value: serde_json::Value = client
                .post(endpoint)
                .form(&[("grant_type", "refresh_token"), ("refresh_token", secret)])
                .send()
                .unwrap()
                .json()
                .unwrap();
            assert_eq!(value["access_token"], "synthetic-access");
            assert_ne!(value["refresh_token"], "synthetic-private-refresh");
        }
        let mut saved: serde_json::Value =
            serde_json::from_slice(&fs::read(conf.join("google-account.json")).unwrap()).unwrap();
        saved["subject"] = "different-account".into();
        fs::write(conf.join("google-account.json"), saved.to_string()).unwrap();
        assert_eq!(
            client
                .post(endpoint)
                .form(&[("grant_type", "refresh_token"), ("refresh_token", secret)])
                .send()
                .unwrap()
                .status()
                .as_u16(),
            400
        );
        let temp_path = bridge.path.clone();
        drop(bridge);
        assert!(!temp_path.exists());
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_dir_all(&runtime.boreal_home).unwrap();
    }
}
