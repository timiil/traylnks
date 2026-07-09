//! Aliyun Drive (阿里云盘) provider via the **web-client / passport** login
//! flow — the same approach the `aligo` library uses.
//!
//! **No app registration is required.** The user scans a QR with the Aliyun
//! Drive app; we poll the passport status endpoint, and on confirm we decode
//! the returned `bizExt` (base64 of gb18030 JSON) to get a `refreshToken`,
//! then exchange it at `/v2/account/token` for an access/refresh token pair.
//!
//! File operations use the `api.aliyundrive.com` web API with the public
//! web-client `CLIENT_ID` and the Android-client headers (no `Bearer` prefix —
//! the raw token goes in `Authorization`).

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use parking_lot::Mutex;
use reqwest::{header, Client, Method};
use serde::Deserialize;

use super::tokens::{self, TokenSet};
use super::{
    ApiResult, AuthChallenge, AuthStart, AuthStatus, CloudError, CloudProvider, RemoteKind,
    RemoteNode,
};

const PROVIDER_ID: &str = "aliyun";
const API_HOST: &str = "https://api.aliyundrive.com";
const AUTH_HOST: &str = "https://auth.aliyundrive.com";
const PASSPORT_HOST: &str = "https://passport.aliyundrive.com";
/// Public web-client id (aligo's; no registration needed).
const CLIENT_ID: &str = "25dzX3vbYqktVxyX";
const ROOT_FILE_ID: &str = "root";
/// Android-client UA — some endpoints (notably download_url) need it.
const UA: &str = "AliApp(AYSD/5.8.0) com.alicloud.databox/37029260 Channel/36176927979800@rimet_android_5.8.0 language/zh-CN /Android Mobile/Xiaomi Redmi";
const X_CANARY: &str = "client=Android,app=adrive,version=v5.8.0";

fn now_unix() -> u64 {
    chrono::Utc::now().timestamp().max(0) as u64
}

fn parse_mtime(s: &str) -> u64 {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.timestamp().max(0) as u64)
        .unwrap_or(0)
}

fn truncate(s: &str, n: usize) -> String {
    let count = s.chars().count();
    if count <= n {
        s.to_string()
    } else {
        let head: String = s.chars().take(n).collect();
        format!("{head}…")
    }
}

/// Mask credential-bearing fields in a JSON blob before it hits the logs.
fn redact_json(s: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(s) {
        Ok(mut v) => {
            redact_value(&mut v);
            v.to_string()
        }
        Err(_) => truncate(s, 500),
    }
}

fn redact_value(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(m) => {
            // Credentials + account-identifying fields — never write to logs.
            for k in ["access_token", "refresh_token", "bizExt", "user_id", "user_name", "nick_name"] {
                if m.contains_key(k) {
                    m[k] = serde_json::Value::String("***".into());
                }
            }
            for (_, vv) in m.iter_mut() {
                redact_value(vv);
            }
        }
        serde_json::Value::Array(a) => {
            for vv in a {
                redact_value(vv);
            }
        }
        _ => {}
    }
}

/// Render `text` into a base64 SVG data URL. The passport `codeContent` is text
/// to encode, not an image; we build the SVG from the module matrix directly so
/// the `image` crate isn't pulled in for QR generation.
fn render_qr_data_url(text: &str) -> Result<String, CloudError> {
    let code = qrcode::QrCode::new(text.as_bytes())
        .map_err(|e| CloudError::Other(format!("qr encode: {e}")))?;
    let w = code.width();
    let colors = code.into_colors();
    let scale = 8u32;
    let dim = (w as u32) * scale;
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{dim}\" height=\"{dim}\" viewBox=\"0 0 {dim} {dim}\"><rect width=\"{dim}\" height=\"{dim}\" fill=\"#ffffff\"/>"
    );
    for (i, c) in colors.iter().enumerate() {
        if matches!(c, qrcode::Color::Dark) {
            let x = (i as u32 % w as u32) * scale;
            let y = (i as u32 / w as u32) * scale;
            svg.push_str(&format!(
                "<rect x=\"{x}\" y=\"{y}\" width=\"{scale}\" height=\"{scale}\" fill=\"#000000\"/>"
            ));
        }
    }
    svg.push_str("</svg>");
    let b64 = base64::engine::general_purpose::STANDARD.encode(svg.as_bytes());
    Ok(format!("data:image/svg+xml;base64,{b64}"))
}

struct PendingAuth {
    /// The `content.data` object from generate.do, re-sent as form fields to query.do.
    data: serde_json::Value,
    /// Captured on CONFIRMED (the pre-token refresh_token from bizExt).
    refresh_token: Option<String>,
}

pub struct AliyunProvider {
    client: Client,
    tokens: Mutex<Option<TokenSet>>,
    pending: Mutex<HashMap<String, PendingAuth>>,
}

impl AliyunProvider {
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .cookie_store(true)
            .user_agent(UA)
            .build()
            .unwrap_or_else(|_| Client::new());
        let tokens = Mutex::new(match tokens::load(PROVIDER_ID) {
            Ok(t) => {
                log::info!("aliyun: stored token {}", if t.is_some() { "present" } else { "absent" });
                t
            }
            Err(e) => {
                log::warn!("aliyun: failed to load stored token: {e}");
                None
            }
        });
        Self {
            client,
            tokens,
            pending: Mutex::new(HashMap::new()),
        }
    }

    fn cached_token(&self) -> Option<TokenSet> {
        self.tokens.lock().clone()
    }

    fn drive_id(&self) -> Result<String, CloudError> {
        self.cached_token()
            .map(|t| t.drive_id)
            .ok_or(CloudError::NotAuthenticated)
    }

    async fn access_token(&self) -> Result<String, CloudError> {
        let tok = self.cached_token().ok_or(CloudError::NotAuthenticated)?;
        if tok.expires_at_unix > now_unix() + 60 {
            Ok(tok.access_token)
        } else {
            Ok(self.refresh(&tok.refresh_token).await?.access_token)
        }
    }

    async fn refresh(&self, refresh_token: &str) -> Result<TokenSet, CloudError> {
        let body =
            serde_json::json!({ "refresh_token": refresh_token, "grant_type": "refresh_token" });
        let raw: TokenResponse = self.post_api("/v2/account/token", &body, None).await?;
        Ok(self.persist_token(raw))
    }

    fn persist_token(&self, raw: TokenResponse) -> TokenSet {
        let t = TokenSet {
            access_token: raw.access_token,
            refresh_token: raw.refresh_token,
            expires_at_unix: now_unix().saturating_add(raw.expires_in.unwrap_or(7200)),
            drive_id: raw.default_drive_id,
            user_name: raw.user_name.filter(|s| !s.is_empty()),
            user_id: raw.user_id,
        };
        if let Err(e) = tokens::save(PROVIDER_ID, &t) {
            log::warn!("aliyun: failed to persist token: {e}");
        }
        *self.tokens.lock() = Some(t.clone());
        t
    }

    // -- HTTP helpers ----------------------------------------------------

    /// `token = None` → no Authorization header (for the token endpoint itself).
    /// `Some(t)` → `Authorization: <t>` (raw, no Bearer — web-client convention).
    async fn post_api<T: for<'de> serde::Deserialize<'de>>(
        &self,
        path: &str,
        body: &serde_json::Value,
        token: Option<&str>,
    ) -> Result<T, CloudError> {
        let url = format!("{API_HOST}{path}");
        log::info!("aliyun → POST {path}  body={}", redact_json(&body.to_string()));
        let mut req = self
            .client
            .request(Method::POST, url)
            .header(header::REFERER, "https://aliyundrive.com")
            .header("x-canary", X_CANARY)
            .json(body);
        if let Some(t) = token {
            req = req.header(header::AUTHORIZATION, t);
        }
        let resp = req.send().await?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| CloudError::Network(e.to_string()))?;
        log::info!("aliyun ← HTTP {status} {path}  body={}", redact_json(&truncate(&text, 1000)));
        if !status.is_success() {
            if let Ok(err) = serde_json::from_str::<AliyunError>(&text) {
                if !err.code.is_empty() {
                    return Err(CloudError::Api { code: err.code, msg: err.message });
                }
            }
            return Err(CloudError::Other(format!("HTTP {status}: {}", truncate(&text, 300))));
        }
        serde_json::from_str(&text).map_err(|e| {
            CloudError::Other(format!("decode response: {e}; body: {}", truncate(&text, 300)))
        })
    }

    async fn list_children(
        &self,
        drive: &str,
        parent: &str,
        type_filter: Option<&str>,
        token: &str,
    ) -> Result<Vec<ListItem>, CloudError> {
        let mut out = Vec::new();
        let mut marker = String::new();
        loop {
            // Minimal body (aligo's shape); filter client-side to avoid schema issues.
            let mut body = serde_json::json!({
                "parent_file_id": parent,
                "limit": 100,
                "marker": marker,
            });
            if !drive.is_empty() {
                body["drive_id"] = serde_json::json!(drive);
            }
            let r: ListResponse = self.post_api("/adrive/v3/file/list", &body, Some(token)).await?;
            out.extend(r.items);
            marker = r.next_marker.unwrap_or_default();
            if marker.is_empty() {
                break;
            }
        }
        if let Some(ty) = type_filter {
            out.retain(|it| it.r#type == ty);
        }
        Ok(out)
    }

    async fn find_child_folder(
        &self,
        drive: &str,
        parent: &str,
        name: &str,
        token: &str,
    ) -> Result<Option<String>, CloudError> {
        let items = self
            .list_children(drive, parent, Some("folder"), token)
            .await?;
        Ok(items
            .into_iter()
            .find(|it| it.r#type == "folder" && it.name == name)
            .map(|it| it.file_id))
    }

    /// Walk `parent` segment by segment; `create` makes missing folders.
    async fn resolve_path(&self, path: &str, create: bool) -> Result<String, CloudError> {
        let token = self.access_token().await?;
        let drive = self.drive_id()?;
        let trimmed = path.trim_matches('/');
        if trimmed.is_empty() {
            return Ok(ROOT_FILE_ID.to_string());
        }
        let mut parent = ROOT_FILE_ID.to_string();
        for seg in trimmed.split('/').filter(|s| !s.is_empty()) {
            let next = self.find_child_folder(&drive, &parent, seg, &token).await?;
            parent = match next {
                Some(id) => id,
                None => {
                    if !create {
                        return Err(CloudError::Other(format!(
                            "folder not found: '{seg}' (in {path})"
                        )));
                    }
                    self.create_folder_under(&drive, &parent, seg, &token)
                        .await?
                }
            };
        }
        Ok(parent)
    }

    async fn create_folder_under(
        &self,
        drive: &str,
        parent: &str,
        name: &str,
        token: &str,
    ) -> Result<String, CloudError> {
        let body = serde_json::json!({
            "drive_id": drive,
            "parent_file_id": parent,
            "name": name,
            "type": "folder",
            "check_name_mode": "ignore", // reuse an existing same-named folder
        });
        let r: serde_json::Value = self
            .post_api("/adrive/v2/file/create", &body, Some(token))
            .await?;
        r.get("file_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| CloudError::Other("create folder: no file_id in response".into()))
    }

    async fn walk_tree(
        &self,
        drive: &str,
        root: &str,
        token: &str,
        out: &mut Vec<RemoteNode>,
    ) -> Result<(), CloudError> {
        let mut stack: Vec<(String, String)> = vec![(root.to_string(), String::new())];
        while let Some((parent, prefix)) = stack.pop() {
            let items = self.list_children(drive, &parent, None, token).await?;
            for it in items {
                let rel = if prefix.is_empty() {
                    it.name.clone()
                } else {
                    format!("{prefix}/{}", it.name)
                };
                let kind = if it.r#type == "folder" {
                    RemoteKind::Folder
                } else {
                    RemoteKind::File
                };
                let file_id = it.file_id.clone();
                out.push(RemoteNode {
                    rel_path: rel.clone(),
                    kind,
                    file_id: file_id.clone(),
                    size: it.size,
                    mtime: parse_mtime(&it.updated_at),
                });
                if kind == RemoteKind::Folder {
                    stack.push((file_id, rel));
                }
            }
        }
        Ok(())
    }

    /// Refresh expired pre-signed upload URLs via `/v2/file/get_upload_url`.
    async fn refresh_upload_urls(
        &self,
        drive: &str,
        file_id: &str,
        upload_id: &str,
        part_numbers: &[u64],
        token: &str,
    ) -> Result<Vec<UploadPart>, CloudError> {
        let list: Vec<serde_json::Value> = part_numbers
            .iter()
            .map(|n| serde_json::json!({ "part_number": n }))
            .collect();
        let body = serde_json::json!({
            "drive_id": drive,
            "file_id": file_id,
            "upload_id": upload_id,
            "part_info_list": list,
        });
        let r: serde_json::Value = self
            .post_api("/v2/file/get_upload_url", &body, Some(token))
            .await?;
        r.get("part_info_list")
            .and_then(|l| serde_json::from_value(l.clone()).ok())
            .ok_or_else(|| CloudError::Other("get_upload_url: no part_info_list in response".into()))
    }
}

#[async_trait]
impl CloudProvider for AliyunProvider {
    async fn start_auth(&self) -> ApiResult<AuthStart> {
        // 1. Init the OAuth session (sets a SESSIONID cookie via the cookie store).
        let _ = self
            .client
            .request(Method::GET, format!("{AUTH_HOST}/v2/oauth/authorize"))
            .query(&[
                ("login_type", "custom"),
                ("response_type", "code"),
                ("redirect_uri", "https://www.aliyundrive.com/sign/callback"),
                ("client_id", CLIENT_ID),
                ("state", "{\"origin\":\"file://\"}"),
            ])
            .send()
            .await;

        // 2. Generate the QR.
        let resp = self
            .client
            .request(
                Method::GET,
                format!("{PASSPORT_HOST}/newlogin/qrcode/generate.do"),
            )
            .query(&[("appName", "aliyun_drive")])
            .send()
            .await?;
        let text = resp
            .text()
            .await
            .map_err(|e| CloudError::Network(e.to_string()))?;
        log::info!("aliyun ← qrcode/generate  body={}", redact_json(&truncate(&text, 600)));
        let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
            CloudError::Other(format!(
                "qrcode/generate decode: {e}; body: {}",
                truncate(&text, 200)
            ))
        })?;
        let data = v
            .get("content")
            .and_then(|c| c.get("data"))
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let code_content = data
            .get("codeContent")
            .and_then(|c| c.as_str())
            .ok_or_else(|| {
                CloudError::Other(format!(
                    "qrcode/generate: no codeContent; body: {}",
                    truncate(&text, 200)
                ))
            })?;
        let data_url = render_qr_data_url(code_content)?;
        // Single-user; key by time so concurrent re-auths don't collide.
        let session = format!("qr-{}", now_unix());
        self.pending.lock().insert(
            session.clone(),
            PendingAuth {
                data,
                refresh_token: None,
            },
        );
        Ok(AuthStart {
            challenge: AuthChallenge::QrImage(data_url),
            session,
            expires_in_secs: 180,
        })
    }

    async fn poll_auth(&self, session: &str) -> ApiResult<AuthStatus> {
        let data = {
            let map = self.pending.lock();
            map.get(session).map(|p| p.data.clone())
        };
        let data = data.ok_or_else(|| CloudError::Other("unknown auth session".into()))?;
        // Echo the generate `data` object back as form fields.
        let form: Vec<(String, String)> = data
            .as_object()
            .map(|m| {
                m.iter()
                    .map(|(k, v)| {
                        let s = match v {
                            serde_json::Value::String(s) => s.clone(),
                            _ => v.to_string(),
                        };
                        (k.clone(), s)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let resp = self
            .client
            .request(
                Method::POST,
                format!("{PASSPORT_HOST}/newlogin/qrcode/query.do"),
            )
            .query(&[("appName", "aliyun_drive")])
            .form(&form)
            .send()
            .await?;
        let text = resp
            .text()
            .await
            .map_err(|e| CloudError::Network(e.to_string()))?;
        log::info!("aliyun ← qrcode/query  body={}", redact_json(&truncate(&text, 600)));
        let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
            CloudError::Other(format!(
                "qrcode/query decode: {e}; body: {}",
                truncate(&text, 200)
            ))
        })?;
        let d = v.get("content").and_then(|c| c.get("data"));
        let raw_status = d
            .and_then(|d| d.get("qrCodeStatus"))
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_uppercase();
        let status = match raw_status.as_str() {
            s if s.contains("CONFIRMED") => AuthStatus::Confirmed,
            s if s.contains("SCANED") || s.contains("SCANNED") => AuthStatus::Scanned,
            s if s.contains("EXPIRED") || s.contains("CANCEL") => AuthStatus::Expired,
            _ => AuthStatus::Waiting,
        };
        if status == AuthStatus::Confirmed {
            // bizExt is base64 of gb18030 JSON. Lossy-UTF8 decode preserves the
            // ASCII refreshToken inside the surrounding Chinese UI strings.
            let biz = d
                .and_then(|d| d.get("bizExt"))
                .and_then(|b| b.as_str())
                .unwrap_or("");
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(biz)
                .unwrap_or_default();
            let lossy = String::from_utf8_lossy(&bytes);
            let rt = serde_json::from_str::<serde_json::Value>(&lossy)
                .ok()
                .and_then(|j| {
                    j.get("pds_login_result")
                        .and_then(|p| p.get("refreshToken"))
                        .and_then(|t| t.as_str())
                        .map(|s| s.to_string())
                });
            if let Some(rt) = rt {
                if let Some(p) = self.pending.lock().get_mut(session) {
                    p.refresh_token = Some(rt);
                }
            }
        }
        Ok(status)
    }

    async fn finalize_auth(&self, session: &str) -> ApiResult<()> {
        let rt = {
            let map = self.pending.lock();
            map.get(session).and_then(|p| p.refresh_token.clone())
        };
        let rt = rt.ok_or_else(|| CloudError::Other("no refresh_token captured".into()))?;
        let body = serde_json::json!({ "refresh_token": rt, "grant_type": "refresh_token" });
        let raw: TokenResponse = self.post_api("/v2/account/token", &body, None).await?;
        self.persist_token(raw);
        self.pending.lock().remove(session);
        Ok(())
    }

    async fn disconnect(&self) -> ApiResult<()> {
        *self.tokens.lock() = None;
        tokens::clear(PROVIDER_ID)?;
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.tokens.lock().is_some()
    }

    fn account(&self) -> Option<super::AccountInfo> {
        self.cached_token()
            .map(|t| super::AccountInfo { user_id: t.user_id, user_name: t.user_name })
    }

    async fn list_folders(&self, path: &str) -> ApiResult<Vec<RemoteNode>> {
        let token = self.access_token().await?;
        let drive = self.drive_id()?;
        let parent = if path.trim_matches('/').is_empty() {
            ROOT_FILE_ID.to_string()
        } else {
            match self.resolve_path(path, false).await {
                Ok(id) => id,
                Err(_) => return Ok(Vec::new()),
            }
        };
        let items = self
            .list_children(&drive, &parent, Some("folder"), &token)
            .await?;
        Ok(items
            .into_iter()
            .map(|it| RemoteNode {
                rel_path: it.name,
                kind: RemoteKind::Folder,
                file_id: it.file_id,
                size: 0,
                mtime: parse_mtime(&it.updated_at),
            })
            .collect())
    }

    async fn resolve_folder(&self, path: &str) -> ApiResult<String> {
        self.resolve_path(path, true).await
    }

    async fn list_tree(&self, root_file_id: &str) -> ApiResult<Vec<RemoteNode>> {
        let token = self.access_token().await?;
        let drive = self.drive_id()?;
        let mut out = Vec::new();
        self.walk_tree(&drive, root_file_id, &token, &mut out)
            .await?;
        Ok(out)
    }

    async fn download(&self, file_id: &str, dest: &Path) -> ApiResult<u64> {
        let token = self.access_token().await?;
        let drive = self.drive_id()?;
        let body = serde_json::json!({ "drive_id": drive, "file_id": file_id });
        let r: serde_json::Value = self
            .post_api("/v2/file/get_download_url", &body, Some(&token))
            .await?;
        let url = r
            .get("url")
            .and_then(|u| u.as_str())
            .ok_or_else(|| CloudError::Other("no download url in response".into()))?;
        let bytes = self.client.get(url).send().await?.bytes().await?;
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(dest, &bytes)?;
        Ok(bytes.len() as u64)
    }

    async fn upload(&self, parent_file_id: &str, local: &Path) -> ApiResult<String> {
        let token = self.access_token().await?;
        let drive = self.drive_id()?;
        let name = local
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .ok_or_else(|| CloudError::Other("upload: local path has no file name".into()))?;
        let data = std::fs::read(local)?;
        let size = data.len() as u64;

        // Step 1 — create (reserve; returns upload URLs or rapid_upload).
        let body = serde_json::json!({
            "drive_id": drive,
            "parent_file_id": parent_file_id,
            "name": name,
            "type": "file",
            "check_name_mode": "auto_rename",
            "size": size,
        });
        let create: serde_json::Value = self
            .post_api("/adrive/v2/file/create", &body, Some(&token))
            .await?;
        let file_id = create
            .get("file_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CloudError::Other("create: no file_id in response".into()))?
            .to_string();

        // 秒传 (instant upload) — already deduplicated server-side; nothing to PUT.
        if create
            .get("rapid_upload")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return Ok(file_id);
        }

        let upload_id = create
            .get("upload_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CloudError::Other("create: no upload_id in response".into()))?;
        // part info may be nested under `part_upload_info` or top-level.
        let part_info = create.get("part_upload_info").unwrap_or(&create);
        let part_size: usize = part_info
            .get("part_size")
            .and_then(|v| v.as_u64())
            .unwrap_or(size) as usize;
        let parts: Vec<UploadPart> = part_info
            .get("part_info_list")
            .and_then(|l| serde_json::from_value(l.clone()).ok())
            .ok_or_else(|| CloudError::Other("create: no part_info_list in response".into()))?;

        // Step 2 — PUT each part. The OSS pre-signed URL must receive a BARE PUT
        // (no Content-Type — adding one breaks the signature → 403). A 403 also
        // means the URL expired → refresh via get_upload_url and retry once.
        let part_numbers: Vec<u64> = parts.iter().map(|p| p.part_number).collect();
        let mut parts = parts;
        let mut refreshed = false;
        let mut i = 0;
        while i < parts.len() {
            let start = i * part_size;
            if start >= data.len() {
                break;
            }
            let end = (start + part_size).min(data.len());
            let url = parts[i].upload_url.clone();
            let resp = self
                .client
                .put(&url)
                .body(data[start..end].to_vec())
                .send()
                .await?;
            if resp.status().is_success() {
                i += 1;
                continue;
            }
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            log::warn!(
                "aliyun upload PUT part {} {} body={}",
                parts[i].part_number,
                status,
                truncate(&body, 500)
            );
            if status.as_u16() == 403 && !refreshed {
                refreshed = true;
                parts = self
                    .refresh_upload_urls(&drive, &file_id, upload_id, &part_numbers, &token)
                    .await?;
                continue; // retry the same part with the fresh URL
            }
            return Err(CloudError::Other(format!(
                "upload part {} failed: HTTP {} {}",
                parts[i].part_number,
                status,
                truncate(&body, 200)
            )));
        }

        // Step 3 — complete.
        let body =
            serde_json::json!({ "drive_id": drive, "file_id": file_id, "upload_id": upload_id });
        let _: serde_json::Value = self
            .post_api("/v2/file/complete", &body, Some(&token))
            .await?;
        Ok(file_id)
    }

    async fn delete_remote(&self, file_id: &str) -> ApiResult<()> {
        let token = self.access_token().await?;
        let drive = self.drive_id()?;
        let body = serde_json::json!({ "drive_id": drive, "file_id": file_id });
        let _: serde_json::Value = self
            .post_api("/v2/recyclebin/trash", &body, Some(&token))
            .await?;
        Ok(())
    }

    async fn create_folder(&self, parent_file_id: &str, name: &str) -> ApiResult<String> {
        let token = self.access_token().await?;
        let drive = self.drive_id()?;
        self.create_folder_under(&drive, parent_file_id, name, &token)
            .await
    }
}

// -- response shapes --------------------------------------------------------

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    default_drive_id: String,
    #[serde(default)]
    user_name: Option<String>,
    #[serde(default)]
    user_id: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "snake_case")]
struct ListResponse {
    #[serde(default)]
    items: Vec<ListItem>,
    #[serde(default)]
    next_marker: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "snake_case")]
struct ListItem {
    #[serde(default)]
    file_id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    r#type: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    updated_at: String,
}

#[derive(Deserialize)]
struct UploadPart {
    #[serde(default)]
    part_number: u64,
    #[serde(default)]
    upload_url: String,
}

#[derive(Deserialize, Default)]
struct AliyunError {
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
}
