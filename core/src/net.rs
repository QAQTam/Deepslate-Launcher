//! 极简 HTTP 辅助：统一 UA / 超时 / 重试。
//!
//! Mojang 与 Fabric 的元数据端点在网络抖动时偶尔会超时或截断响应
//! （`error decoding response body`），因此重试必须覆盖「发送请求」和
//! 「读取响应体」两个阶段。

use std::time::Duration;

use serde::de::DeserializeOwned;

pub const USER_AGENT: &str =
    "DeepslateLauncher/0.1.0 (Rust; +https://github.com/QAQTam/Deepslate-Launcher)";

const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const ATTEMPTS: u32 = 4;

pub fn client() -> Result<reqwest::Client, String> {
    builder()
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))
}

/// 部分 Akamai 站点（例如 minecraft.net 的 EULA 页面）会重置 HTTP/2 流
/// （`INTERNAL_ERROR`），但 HTTP/1.1 正常。仅供这类静态页面抓取使用。
pub fn client_http1() -> Result<reqwest::Client, String> {
    client_http1_with_user_agent(USER_AGENT)
}

pub fn client_http1_with_user_agent(user_agent: &str) -> Result<reqwest::Client, String> {
    builder()
        .user_agent(user_agent)
        .http1_only()
        .build()
        .map_err(|e| format!("创建 HTTP/1.1 客户端失败: {e}"))
}

fn builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(REQUEST_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
}

/// 单次请求。4xx 直接返回不可重试的错误。
async fn send_once(url: &str, client: &reqwest::Client) -> Result<reqwest::Response, String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("请求失败: {url}: {e}"))?;
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    if status.as_u16() == 404 {
        return Err(format!("资源不存在 (404): {url}"));
    }
    if status.is_client_error() {
        return Err(format!("HTTP {status}: {url}"));
    }
    Err(format!("HTTP {status}: {url}"))
}

/// 带重试地读取完整响应体。错误不可重试时（4xx）立即返回。
pub async fn get_bytes(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let mut last_error = String::new();
    for attempt in 0..ATTEMPTS {
        let result = match send_once(url, client).await {
            Ok(response) => response
                .bytes()
                .await
                .map(|b| b.to_vec())
                .map_err(|e| format!("读取响应失败: {url}: {e}")),
            Err(e) => Err(e),
        };
        match result {
            Ok(bytes) => return Ok(bytes),
            Err(e) => {
                let permanent = e.contains("(404)") || e.starts_with("HTTP 4");
                last_error = e;
                if permanent {
                    return Err(last_error);
                }
            }
        }
        if attempt + 1 < ATTEMPTS {
            tokio::time::sleep(Duration::from_millis(700 * (attempt as u64 + 1))).await;
        }
    }
    Err(last_error)
}

pub async fn get_text(client: &reqwest::Client, url: &str) -> Result<String, String> {
    let bytes = get_bytes(client, url).await?;
    String::from_utf8(bytes).map_err(|e| format!("响应不是合法 UTF-8: {url}: {e}"))
}

/// 单次读取，不套用下载器的多次重试。EULA 这类静态页面由调用方控制
/// 超时并允许用户手动重试，避免断网时把首次启动卡上几分钟。
pub async fn get_text_once(client: &reqwest::Client, url: &str) -> Result<String, String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("请求失败: {url}: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("HTTP {status}: {url}"));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("读取响应失败: {url}: {e}"))?;
    String::from_utf8(bytes.to_vec()).map_err(|e| format!("响应不是合法 UTF-8: {url}: {e}"))
}

pub async fn get_json<T: DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
) -> Result<T, String> {
    let text = get_text(client, url).await?;
    serde_json::from_str(&text).map_err(|e| format!("解析 JSON 失败: {url}: {e}"))
}
