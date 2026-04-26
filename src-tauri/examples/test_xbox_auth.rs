// 测试 Xbox 认证 - 循环测试直到成功
// 运行方式: cargo run --example test_xbox_auth <ms_access_token>

use serde::{Deserialize, Serialize};

const XBOX_AUTH_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";

#[derive(Debug, Serialize, Deserialize)]
struct XboxAuthResponse {
    IssueInstant: String,
    NotAfter: String,
    Token: String,
    #[serde(rename = "DisplayClaims")]
    display_claims: XboxDisplayClaims,
}

#[derive(Debug, Serialize, Deserialize)]
struct XboxDisplayClaims {
    xui: Option<Vec<XboxClaim>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct XboxClaim {
    uhs: String,
}

#[tokio::main]
async fn main() {
    let ms_token = std::env::args().nth(1).expect("Usage: test_xbox_auth <ms_access_token>");

    println!("MS Token: {}", ms_token);
    println!("开始循环测试 Xbox 认证...\n");

    let mut attempts = 0;
    let max_attempts = 10;

    while attempts < max_attempts {
        attempts += 1;
        println!("=== 尝试 {} ===", attempts);

        match authenticate_xbox(&ms_token).await {
            Ok(response) => {
                println!("\n✅ Xbox 认证成功!");
                println!("Token: {}...", &response.Token[..50.min(response.Token.len())]);
                if let Some(xui) = response.display_claims.xui {
                    if let Some(claim) = xui.first() {
                        println!("UHS: {}", claim.uhs);
                    }
                }
                return;
            }
            Err(e) => {
                println!("❌ Xbox 认证失败: {}", e);
            }
        }

        if attempts < max_attempts {
            println!("等待 2 秒后重试...\n");
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }

    println!("\n所有尝试都失败了");
}

async fn authenticate_xbox(ms_access_token: &str) -> Result<XboxAuthResponse, String> {
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    // 尝试不同的 RelyingParty 值
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
    println!("请求体: {}", body);

    let resp = client
        .post(XBOX_AUTH_URL)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .header("x-xbl-client-type", "device")
        .header("x-xbl-client-version", "1.0.0.0")
        .header("Content-Length", body_bytes.len().to_string())
        .body(body_bytes)
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    let status = resp.status();
    println!("状态码: {}", status);

    let text = resp.text().await.unwrap_or_default();
    println!("响应体: {}", text);

    if !status.is_success() {
        return Err(format!("Xbox 认证失败 ({}): {}", status, text));
    }

    serde_json::from_str(&text).map_err(|e| format!("JSON 解析失败: {}", e))
}