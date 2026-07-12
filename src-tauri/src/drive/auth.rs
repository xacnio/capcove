use super::{
    DriveClient, TokenResponse, Tokens, AUTH_URL, TOKEN_URL, token_store_write, urlencode,
    MAIN_TOKEN_SLOT, YT_TOKEN_SLOT,
};
use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;
use zeroize::Zeroize;

impl DriveClient {
    pub(super) fn store_tokens(&self, tokens: Tokens) -> Result<()> {
        // Serialize without photo (photo is cached separately).
        let mut tokens_no_photo = tokens.clone();
        tokens_no_photo.account_photo = None;
        let mut bytes = serde_json::to_vec(&tokens_no_photo)?;
        drop(tokens_no_photo); // zeroize the clone
        token_store_write(MAIN_TOKEN_SLOT, &bytes, &self.tokens_file_path)?;
        bytes.zeroize();

        // Photo cache — not sensitive, too large to encrypt.
        if let Some(ref photo) = tokens.account_photo {
            let _ = std::fs::write(&self.photo_cache_path, photo);
        } else {
            let _ = std::fs::remove_file(&self.photo_cache_path);
        }

        *self.tokens.lock().unwrap() = Some(tokens); // old value is ZeroizeOnDrop'd here
        Ok(())
    }

    pub(super) fn store_yt_tokens(&self, tokens: Tokens) -> Result<()> {
        let mut bytes = serde_json::to_vec(&tokens)?;
        token_store_write(YT_TOKEN_SLOT, &bytes, &self.yt_tokens_file_path)?;
        bytes.zeroize();
        *self.yt_tokens.lock().unwrap() = Some(tokens);
        Ok(())
    }

    /// Main-account OAuth: Drive scopes only, so the stored identity is always
    /// the user's real account. Returns the email from the Drive `about` endpoint.
    pub async fn authorize(
        &self,
        client_id: &str,
        client_secret: &str,
        login_hint: Option<&str>,
        open_browser: impl FnOnce(String) + Send + 'static,
    ) -> Result<String> {
        let scope = "https://www.googleapis.com/auth/drive.file";
        let mut tokens = self.oauth_flow(client_id, client_secret, scope, "consent", login_hint, open_browser).await?;

        if let Ok(resp) = self
            .http
            .get(format!("{}/about?fields=user(emailAddress,displayName,photoLink)", super::API))
            .bearer_auth(&tokens.access_token)
            .send()
            .await
        {
            #[derive(serde::Deserialize)]
            struct About { user: Option<AboutUser> }
            #[derive(serde::Deserialize)]
            struct AboutUser {
                #[serde(rename = "emailAddress")] email_address: Option<String>,
                #[serde(rename = "displayName")] display_name: Option<String>,
                #[serde(rename = "photoLink")] photo_link: Option<String>,
            }
            if let Ok(about) = resp.json::<About>().await {
                if let Some(u) = about.user {
                    tokens.account_email = u.email_address;
                    tokens.account_name = u.display_name;
                    if let Some(url) = u.photo_link {
                        let sized_url = if let Some(pos) = url.rfind("=s") {
                            format!("{}=s96", &url[..pos])
                        } else {
                            format!("{url}=s96")
                        };
                        if let Ok(img_resp) = self.http.get(&sized_url).send().await {
                            if img_resp.status().is_success() {
                                let mime = img_resp.headers().get("content-type")
                                    .and_then(|v| v.to_str().ok())
                                    .unwrap_or("image/jpeg")
                                    .to_string();
                                if let Ok(bytes) = img_resp.bytes().await {
                                    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                                    tokens.account_photo = Some(format!("data:{mime};base64,{b64}"));
                                }
                            }
                        }
                    }
                }
            }
        }

        let email = tokens.account_email.clone().unwrap_or_default();
        self.store_tokens(tokens)?;
        Ok(email)
    }

    /// Dedicated YouTube-channel OAuth: YouTube scopes only, no Drive.
    /// `login_hint` pins the flow to the already-connected account. The token
    /// lands in a separate slot, leaving Drive sync on the main token.
    pub async fn authorize_youtube(
        &self,
        client_id: &str,
        client_secret: &str,
        login_hint: Option<&str>,
        open_browser: impl FnOnce(String) + Send + 'static,
    ) -> Result<()> {
        // `userinfo.email` captures which Google account owns this channel, so
        // "Open in YouTube Studio" can deep-link with `authuser=<that email>`
        // instead of landing on the browser's default (wrong) account.
        let scope = "https://www.googleapis.com/auth/youtube.upload https://www.googleapis.com/auth/youtube https://www.googleapis.com/auth/userinfo.email";
        let mut tokens = self.oauth_flow(client_id, client_secret, scope, "consent", login_hint, open_browser).await?;

        if let Ok(resp) = self
            .http
            .get("https://www.googleapis.com/oauth2/v2/userinfo")
            .bearer_auth(&tokens.access_token)
            .send()
            .await
        {
            #[derive(serde::Deserialize)]
            struct UserInfo { email: Option<String> }
            if let Ok(u) = resp.json::<UserInfo>().await {
                tokens.account_email = u.email;
            }
        }

        self.store_yt_tokens(tokens)?;
        Ok(())
    }

    /// The shared PKCE browser flow: consent URL → loopback redirect →
    /// code/token exchange. Returns tokens with no identity fields filled.
    async fn oauth_flow(
        &self,
        client_id: &str,
        client_secret: &str,
        scope: &str,
        prompt: &str,
        login_hint: Option<&str>,
        open_browser: impl FnOnce(String) + Send + 'static,
    ) -> Result<Tokens> {
        if client_id.trim().is_empty() {
            bail!("Google Client ID is empty.");
        }
        let client_id = client_id.trim().to_string();
        let client_secret = client_secret.trim().to_string();

        let verifier: String = {
            use rand::Rng;
            let mut rng = rand::thread_rng();
            (0..64)
                .map(|_| {
                    let chars = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
                    chars[rng.gen_range(0..chars.len())] as char
                })
                .collect()
        };
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()));

        let listener = TcpListener::bind("127.0.0.1:0").context("failed to open loopback port")?;
        let port = listener.local_addr()?.port();
        let redirect_uri = format!("http://127.0.0.1:{port}");

        let state: String = {
            use rand::Rng;
            let mut rng = rand::thread_rng();
            (0..16).map(|_| char::from(rng.gen_range(b'a'..=b'z'))).collect()
        };

        let mut auth_url = format!(
            "{AUTH_URL}?client_id={}&redirect_uri={}&response_type=code&scope={}&code_challenge={}&code_challenge_method=S256&state={}&access_type=offline&prompt={}",
            urlencode(&client_id),
            urlencode(&redirect_uri),
            urlencode(scope),
            challenge,
            state,
            urlencode(prompt),
        );
        if let Some(hint) = login_hint {
            auth_url.push_str(&format!("&login_hint={}", urlencode(hint)));
        }
        open_browser(auth_url);

        let cancel_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        *self.auth_cancel.lock().unwrap() = Some(cancel_flag.clone());
        struct ClearCancelOnDrop<'a>(&'a DriveClient);
        impl Drop for ClearCancelOnDrop<'_> {
            fn drop(&mut self) {
                *self.0.auth_cancel.lock().unwrap() = None;
            }
        }
        let _clear_cancel = ClearCancelOnDrop(self);

        let expected_state = state.clone();
        let code = tokio::task::spawn_blocking(move || -> Result<String> {
            listener.set_nonblocking(true).ok();
            let deadline = std::time::Instant::now() + Duration::from_secs(300);
            let mut stream = loop {
                if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
                    bail!("cancelled");
                }
                if std::time::Instant::now() >= deadline {
                    bail!("timed out waiting for browser approval");
                }
                match listener.accept() {
                    Ok((s, _)) => break s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(200));
                    }
                    Err(e) => return Err(e).context("failed to receive redirect"),
                }
            };
            stream.set_nonblocking(false).ok();
            stream.set_read_timeout(Some(Duration::from_secs(180))).ok();
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf)?;
            let request = String::from_utf8_lossy(&buf[..n]).to_string();
            let first_line = request.lines().next().unwrap_or_default();
            let path = first_line.split_whitespace().nth(1).unwrap_or_default();
            let query = path.split_once('?').map(|(_, q)| q).unwrap_or_default();
            let mut code = None;
            let mut got_state = None;
            for pair in query.split('&') {
                if let Some((k, v)) = pair.split_once('=') {
                    match k {
                        "code" => code = Some(v.to_string()),
                        "state" => got_state = Some(v.to_string()),
                        _ => {}
                    }
                }
            }
            let body = "<html><body style=\"font-family:sans-serif;background:#1e1e1e;color:#eee;display:flex;align-items:center;justify-content:center;height:100vh\"><div><h2>Capcove connected \u{2713}</h2><p>You can close this window.</p></div></body></html>";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(), body
            );
            stream.write_all(response.as_bytes()).ok();
            if got_state.as_deref() != Some(expected_state.as_str()) {
                bail!("state validation failed");
            }
            code.ok_or_else(|| anyhow!("authorization code not received (user may have denied access)"))
        })
        .await??;

        let params = vec![
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("code", code.as_str()),
            ("code_verifier", verifier.as_str()),
            ("grant_type", "authorization_code"),
            ("redirect_uri", redirect_uri.as_str()),
        ];
        let resp = self.http.post(TOKEN_URL).form(&params).send().await?;
        if !resp.status().is_success() {
            bail!("failed to get token: {}", resp.text().await.unwrap_or_default());
        }
        let tr: TokenResponse = resp.json().await?;
        let refresh = tr.refresh_token.ok_or_else(|| anyhow!("refresh token not received"))?;

        Ok(Tokens {
            access_token: tr.access_token,
            refresh_token: refresh,
            expires_at: chrono::Utc::now().timestamp() + tr.expires_in - 60,
            account_email: None,
            account_name: None,
            account_photo: None,
        })
    }

    /// Called once on startup. If the stored token is expired, attempts a refresh.
    /// - Network error (no internet) → keeps tokens intact, returns Ok(())
    /// - Auth error (invalid_grant / 4xx) → clears tokens so the user sees "not connected"
    pub async fn validate_on_startup(&self, client_id: &str, client_secret: &str) {
        let (needs_refresh, refresh_token) = {
            let guard = self.tokens.lock().unwrap();
            match guard.as_ref() {
                None => return, // not connected at all, nothing to do
                Some(t) => (
                    t.expires_at <= chrono::Utc::now().timestamp(),
                    t.refresh_token.clone(),
                ),
            }
        };

        if !needs_refresh {
            return;
        }

        let params = vec![
            ("client_id", client_id.trim()),
            ("client_secret", client_secret.trim()),
            ("refresh_token", refresh_token.as_str()),
            ("grant_type", "refresh_token"),
        ];

        let resp = match self.http.post(TOKEN_URL).form(&params)
            .timeout(Duration::from_secs(8))
            .send().await
        {
            Ok(r)  => r,
            Err(e) => {
                // Connection-level error: DNS failure, timeout, refused → treat as offline
                log::warn!("Drive startup validation: network error (keeping tokens): {e}");
                return;
            }
        };

        if resp.status().is_success() {
            if let Ok(tr) = resp.json::<TokenResponse>().await {
                let mut guard = self.tokens.lock().unwrap();
                if let Some(t) = guard.as_mut() {
                    t.access_token = tr.access_token;
                    t.expires_at = chrono::Utc::now().timestamp() + tr.expires_in - 60;
                    let snapshot = t.clone();
                    drop(guard);
                    let _ = self.store_tokens(snapshot);
                }
            }
        } else {
            // Auth error: token revoked or expired beyond recovery → disconnect
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            log::warn!("Drive startup validation: auth error {status} ({body}), clearing tokens");
            self.disconnect();
        }
    }

    pub(super) async fn access_token(&self, client_id: &str, client_secret: &str) -> Result<String> {
        let (needs_refresh, refresh_token, current) = {
            let guard = self.tokens.lock().unwrap();
            let t = guard.as_ref().ok_or_else(|| anyhow!("Google Drive account not connected"))?;
            (
                t.expires_at <= chrono::Utc::now().timestamp(),
                t.refresh_token.clone(),
                t.access_token.clone(),
            )
        };
        if !needs_refresh {
            return Ok(current);
        }
        let params = vec![
            ("client_id", client_id.trim()),
            ("client_secret", client_secret.trim()),
            ("refresh_token", refresh_token.as_str()),
            ("grant_type", "refresh_token"),
        ];
        let resp = self.http.post(TOKEN_URL).form(&params).send().await?;
        if !resp.status().is_success() {
            bail!(
                "failed to refresh token (you may need to reconnect the account): {}",
                resp.text().await.unwrap_or_default()
            );
        }
        let tr: TokenResponse = resp.json().await?;
        let new_tokens = {
            let mut guard = self.tokens.lock().unwrap();
            let t = guard.as_mut().unwrap();
            t.access_token = tr.access_token.clone();
            t.expires_at = chrono::Utc::now().timestamp() + tr.expires_in - 60;
            t.clone()
        };
        self.store_tokens(new_tokens)?;
        Ok(tr.access_token)
    }

    /// Access token for YouTube API calls: the dedicated channel token when
    /// one is connected (refreshing it as needed), the main token otherwise.
    pub(super) async fn yt_access_token(&self, client_id: &str, client_secret: &str) -> Result<String> {
        let snapshot = {
            let guard = self.yt_tokens.lock().unwrap();
            guard.as_ref().map(|t| (
                t.expires_at <= chrono::Utc::now().timestamp(),
                t.refresh_token.clone(),
                t.access_token.clone(),
            ))
        };
        let Some((needs_refresh, refresh_token, current)) = snapshot else {
            // No dedicated channel token — YouTube rides on the main account.
            return self.access_token(client_id, client_secret).await;
        };
        if !needs_refresh {
            return Ok(current);
        }
        let params = vec![
            ("client_id", client_id.trim()),
            ("client_secret", client_secret.trim()),
            ("refresh_token", refresh_token.as_str()),
            ("grant_type", "refresh_token"),
        ];
        let resp = self.http.post(TOKEN_URL).form(&params).send().await?;
        if !resp.status().is_success() {
            bail!(
                "failed to refresh the YouTube channel token (reconnect the channel): {}",
                resp.text().await.unwrap_or_default()
            );
        }
        let tr: TokenResponse = resp.json().await?;
        let new_tokens = {
            let mut guard = self.yt_tokens.lock().unwrap();
            let t = guard.as_mut().unwrap();
            t.access_token = tr.access_token.clone();
            t.expires_at = chrono::Utc::now().timestamp() + tr.expires_in - 60;
            t.clone()
        };
        self.store_yt_tokens(new_tokens)?;
        Ok(tr.access_token)
    }
}
