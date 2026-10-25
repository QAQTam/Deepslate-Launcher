use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::events::CoreEvent;

// ============================================================
// Microsoft OAuth 2.0 via login.live.com (Authorization Code Flow)
// 授权页在系统浏览器中打开，用户把回调 URL（含 code）粘贴回启动器，
// 由 login_with_code 完成后续 token 交换链
// ============================================================

// -------------------- 认证 API 端点 --------------------

const LOGIN_AUTHORIZE_URL: &str = "https://login.live.com/oauth20_authorize.srf";
const LOGIN_TOKEN_URL: &str = "https://login.live.com/oauth20_token.srf";
const XBOX_AUTH_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS_AUTH_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MC_AUTH_URL: &str = "https://api.minecraftservices.com/authentication/login_with_xbox";
const MC_PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";

// Minecraft Launcher 公共客户端 ID
const CLIENT_ID: &str = "00000000402b5328";
const REDIRECT_URI: &str = "https://login.live.com/oauth20_desktop.srf";
const SCOPE: &str = "service%3A%3Auser.auth.xboxlive.com%3A%3AMBI_SSL";

// -------------------- 数据结构 --------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct MsTokenResponse {
    pub token_type: String,
    pub expires_in: u32,
    pub access_token: String,
    pub refresh_token: String,
    pub user_id: String,
    pub foci: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct XboxAuthResponse {
    pub IssueInstant: String,
    pub NotAfter: String,
    pub Token: String,
    #[serde(rename = "DisplayClaims")]
    pub display_claims: XboxDisplayClaims,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct XboxDisplayClaims {
    pub xui: Option<Vec<XboxClaim>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct XboxClaim {
    pub uhs: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct XboxStsResponse {
    pub IssueInstant: String,
    pub NotAfter: String,
    pub Token: String,
    #[serde(rename = "DisplayClaims")]
    pub display_claims: XboxDisplayClaims,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct McTokenResponse {
    pub username: String,
    pub roles: Vec<String>,
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct McProfile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub skins: Vec<serde_json::Value>,
    #[serde(default)]
    pub capes: Vec<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
pub enum AuthMethod {
    #[default]
    Offline,
    Microsoft,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OnlineProfile {
    pub username: String,
    pub uuid: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
}

#[derive(Debug, Serialize, Clone)]
pub struct LoginProgress {
    pub step: String,
    pub message: String,
}

// -------------------- 核心认证流程 --------------------

/// 使用授权码完成 Microsoft 登录：MS token → Xbox → XSTS → Minecraft → 档案
pub async fn login_with_code(code: &str, events: &mpsc::Sender<CoreEvent>) -> Result<OnlineProfile, String> {
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    // Step 1: 用 code 换取 MS token
    emit_progress(events, "exchanging_ms_token", "正在验证登录...").await;
    let ms_token = exchange_code_for_token(&client, code).await?;

    // Step 2: Xbox 认证
    emit_progress(events, "exchanging_xbox", "正在连接 Xbox 服务...").await;
    let xbox_token = authenticate_xbox(&client, &ms_token.access_token).await?;

    // Step 3: XSTS 认证
    emit_progress(events, "exchanging_xsts", "正在验证 Xbox 权限...").await;
    let xsts_token = exchange_xsts(&client, &xbox_token.Token).await?;

    // Step 4: Minecraft token
    emit_progress(events, "exchanging_minecraft", "正在连接 Minecraft 服务...").await;
    let uhs = xbox_token
        .display_claims
        .xui
        .as_ref()
        .and_then(|xui| xui.first())
        .map(|c| c.uhs.as_str())
        .ok_or("未找到 Xbox 用户标识")?;
    let mc_token = login_minecraft(&client, uhs, &xsts_token.Token).await?;

    // Step 5: 获取 Minecraft 档案
    emit_progress(events, "fetching_profile", "正在获取玩家档案...").await;
    let mc_profile = get_minecraft_profile(&client, &mc_token.access_token).await?;

    let expires_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
        + mc_token.expires_in as i64;

    let profile = OnlineProfile {
        username: mc_profile.name,
        uuid: mc_profile.id,
        access_token: mc_token.access_token,
        refresh_token: ms_token.refresh_token,
        expires_at,
    };

    emit_progress(events, "completed", &format!("登录成功: {}", profile.username)).await;
    Ok(profile)
}

// -------------------- 授权 URL --------------------

pub fn get_auth_url() -> String {
    construct_auth_url()
}

fn construct_auth_url() -> String {
    let redirect_uri_encoded = url::form_urlencoded::byte_serialize(REDIRECT_URI.as_bytes()).collect::<String>();

    format!(
        "{}?client_id={}&response_type=code&scope={}&redirect_uri={}",
        LOGIN_AUTHORIZE_URL,
        CLIENT_ID,
        SCOPE,
        redirect_uri_encoded
    )
}

// -------------------- 认证步骤 --------------------

async fn authenticate_xbox(client: &reqwest::Client, ms_access_token: &str) -> Result<XboxAuthResponse, String> {
    let body = serde_json::json!({
        "Properties": {
            "AuthMethod": "RPS",
            "SiteName": "user.auth.xboxlive.com",
            "RpsTicket": ms_access_token
        },
        "RelyingParty": "http://auth.xboxlive.com",
        "TokenType": "JWT"
    });

    let body_bytes = body.to_string().into_bytes();

    let resp = client
        .post(XBOX_AUTH_URL)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .body(body_bytes)
        .send()
        .await
        .map_err(|e| format!("Xbox 认证请求失败: {}", e))?;

    let status = resp.status();
    let text = resp.text().await.map_err(|e| format!("读取响应失败: {}", e))?;

    if !status.is_success() {
        return Err(format!("Xbox 认证失败 ({}): {}", status, text));
    }

    let xbox_resp: XboxAuthResponse = serde_json::from_str(&text)
        .map_err(|e| format!("解析 Xbox 响应失败: {}", e))?;

    Ok(xbox_resp)
}

async fn exchange_xsts(client: &reqwest::Client, xbox_token: &str) -> Result<XboxStsResponse, String> {
    let body = serde_json::json!({
        "Properties": {
            "SandboxId": "RETAIL",
            "UserTokens": [xbox_token]
        },
        "RelyingParty": "rp://api.minecraftservices.com/",
        "TokenType": "JWT"
    });

    let resp = client
        .post(XSTS_AUTH_URL)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("XSTS 认证请求失败: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("XSTS 认证失败 ({}): {}", status, text));
    }

    let xsts_resp: XboxStsResponse = resp
        .json()
        .await
        .map_err(|e| format!("解析 XSTS 响应失败: {}", e))?;

    println!("XSTS 认证成功");
    Ok(xsts_resp)
}

async fn login_minecraft(client: &reqwest::Client, uhs: &str, xsts_token: &str) -> Result<McTokenResponse, String> {
    let body = serde_json::json!({
        "identityToken": format!("XBL3.0 x={};{}", uhs, xsts_token)
    });

    let resp = client
        .post(MC_AUTH_URL)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Minecraft 登录请求失败: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Minecraft 登录失败 ({}): {}", status, text));
    }

    let mc_resp: McTokenResponse = resp
        .json()
        .await
        .map_err(|e| format!("解析 Minecraft 响应失败: {}", e))?;

    println!("Minecraft 登录成功");
    Ok(mc_resp)
}

async fn get_minecraft_profile(client: &reqwest::Client, access_token: &str) -> Result<McProfile, String> {
    let resp = client
        .get(MC_PROFILE_URL)
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await
        .map_err(|e| format!("获取 Minecraft 档案失败: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        if status.as_u16() == 404 {
            return Err("此 Microsoft 账户未绑定 Minecraft Java Edition".to_string());
        }
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("获取档案失败 ({}): {}", status, text));
    }

    let profile: McProfile = resp
        .json()
        .await
        .map_err(|e| format!("解析 Minecraft 档案失败: {}", e))?;

    println!("Minecraft 档案获取成功: {} ({})", profile.name, profile.id);
    Ok(profile)
}

async fn exchange_code_for_token(client: &reqwest::Client, code: &str) -> Result<MsTokenResponse, String> {
    let redirect_uri_encoded = url::form_urlencoded::byte_serialize(REDIRECT_URI.as_bytes()).collect::<String>();
    let body = format!(
        "client_id={}&code={}&grant_type=authorization_code&redirect_uri={}&scope={}",
        CLIENT_ID,
        code,
        redirect_uri_encoded,
        SCOPE
    );

    let resp = client
        .post(LOGIN_TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|e| format!("获取 MS Token 失败: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("MS Token 请求失败 ({}): {}", status, text));
    }

    let token_resp: MsTokenResponse = resp
        .json()
        .await
        .map_err(|e| format!("解析 MS Token 失败: {}", e))?;

    Ok(token_resp)
}

async fn emit_progress(events: &mpsc::Sender<CoreEvent>, step: &str, message: &str) {
    let progress = LoginProgress {
        step: step.to_string(),
        message: message.to_string(),
    };
    let _ = events.send(CoreEvent::AuthProgress(progress)).await;
}
