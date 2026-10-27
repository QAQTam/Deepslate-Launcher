//! Minecraft EULA：抓取、解析、缓存与本地接受标记。
//!
//! 法务文本不能在启动器里手写/截断，否则很容易与官方页面漂移。这里直接
//! 抓取官方 EULA 页面，从 `#main-content` 提取可见正文，并把原始 HTML、
//! 纯文本和 SHA-1 一起落盘，方便审计“展示的是哪一版文本”。
//!
//! GUI 的接受状态还会把正文 SHA-1 写入 `legal/eula-accepted.json`；启动时
//! [`check_for_update`] 对比当前官方正文，协议实质更新会让旧同意失效。
//!
//! 接受标记复用服务端同款格式的 `<root>/eula.txt`：
//!
//! ```text
//! eula=true
//! ```

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use scraper::{ElementRef, Html, Node, Selector};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

use crate::config::AppConfig;
use crate::net;

/// 官方 EULA 页面（`/eula` 会 302 到这里）。
pub const EULA_URL: &str = "https://www.minecraft.net/en-us/eula";

/// minecraft.net 的 Akamai 规则会卡住带 `+https://...` 的 UA；这里用与
/// 下载器不同的、干净的 UA，避免把法务页面请求伪装成浏览器。
const EULA_USER_AGENT: &str = "DeepslateLauncher/0.1.0 (EULA viewer)";

/// 与 Minecraft 服务端 `eula.txt` 完全一致的接受内容。
pub const EULA_MARKER_CONTENTS: &str = "eula=true\n";

const LEGAL_DIR: &str = "legal";
const TEXT_FILE: &str = "eula.txt";
const HTML_FILE: &str = "eula.html";
const META_FILE: &str = "eula.json";
const ACCEPTANCE_FILE: &str = "eula-accepted.json";

#[derive(Debug, Clone)]
pub struct EulaDocument {
    /// 从 `#main-content` 提取的纯文本；段落/标题/列表按行保留。
    pub text: String,
    pub source_url: String,
    /// Unix 时间戳（秒）。避免为了一个字段引入日期库。
    pub fetched_at: u64,
    /// 纯文本的 SHA-1（十六进制）。
    pub sha1: String,
    /// 在线抓取失败、退回本地缓存时的提示；不影响展示/接受。
    pub warning: Option<String>,
}

impl EulaDocument {
    pub fn summary(&self) -> String {
        format!(
            "来源：{}  ·  抓取时间（Unix）：{}  ·  SHA-1：{}",
            self.source_url, self.fetched_at, self.sha1
        )
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct EulaMeta {
    url: String,
    fetched_at: u64,
    sha1: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EulaAcceptance {
    sha1: String,
    source_url: String,
    accepted_at: u64,
}

/// 启动时比较“接受过的哈希”和“当前官方正文哈希”的结果。
#[derive(Debug, Clone)]
pub enum EulaUpdate {
    /// 本机还没有接受标记，不需要做更新比较。
    NotAccepted,
    /// 协议正文未变化。
    Unchanged,
    /// 协议正文已变化；`document` 是当前版本，必须重新阅读并接受。
    Changed(EulaDocument),
}

/// 全局接受标记：`<root>/eula.txt`。服务端目录里的 `eula.txt` 仍由
/// `server::install/start` 自己写；两者格式一致。
pub fn marker_path(config: &AppConfig) -> PathBuf {
    PathBuf::from(&config.root_dir).join("eula.txt")
}

pub fn is_accepted(config: &AppConfig) -> bool {
    let text = fs::read_to_string(marker_path(config)).unwrap_or_default();
    text.lines().any(|line| {
        let line = line.trim();
        line.strip_prefix("eula=")
            .map(|value| value.trim().eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    })
}

/// 仅写入全局 `eula.txt`，不建立正文版本绑定。仅供不知道当前正文的 CLI
/// `--accept-eula` 使用；GUI 应使用 [`accept_document`] 绑定实际展示的哈希。
pub fn accept(config: &AppConfig) -> Result<(), String> {
    write_marker(config)
}

/// 记录“用户接受的是哪一版”。调用方必须传入刚展示过的正文哈希。
pub fn accept_document(config: &AppConfig, sha1: &str) -> Result<(), String> {
    if sha1.trim().is_empty() {
        return Err("EULA 正文哈希为空，拒绝记录接受状态".to_string());
    }
    // 先写版本绑定，再写服务端兼容的 eula.txt；任一步失败都不能留下
    // “已经同意但不知道同意哪一版”的中间状态。
    record_acceptance(config, sha1)?;
    write_marker(config)
}

fn write_marker(config: &AppConfig) -> Result<(), String> {
    let path = marker_path(config);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("创建 EULA 标记目录失败 {}: {e}", parent.display()))?;
    }
    write_atomic(&path, EULA_MARKER_CONTENTS.as_bytes())
        .map_err(|e| format!("写入 EULA 标记失败 {}: {e}", path.display()))
}

pub fn cache_dir(config: &AppConfig) -> PathBuf {
    PathBuf::from(&config.root_dir).join(LEGAL_DIR)
}

pub fn cached_text_path(config: &AppConfig) -> PathBuf {
    cache_dir(config).join(TEXT_FILE)
}

pub fn acceptance_path(config: &AppConfig) -> PathBuf {
    cache_dir(config).join(ACCEPTANCE_FILE)
}

/// 读取用户接受过的正文哈希；旧版本只有 `eula.txt` 时返回 `None`。
pub fn accepted_sha1(config: &AppConfig) -> Option<String> {
    let raw = fs::read_to_string(acceptance_path(config)).ok()?;
    let acceptance: EulaAcceptance = serde_json::from_str(&raw).ok()?;
    (!acceptance.sha1.trim().is_empty()).then_some(acceptance.sha1)
}

/// 读取上次成功抓取的文本。缺少元数据也能用，只是摘要不完整。
pub fn load_cached(config: &AppConfig) -> Option<EulaDocument> {
    let text = fs::read_to_string(cached_text_path(config)).ok()?;
    if text.trim().is_empty()
        || !text.contains("Minecraft End(er)-User License Agreement (“EULA”)")
        || !text.contains("COMPANY INFORMATION")
        || text.chars().count() < 5_000
    {
        return None;
    }
    let computed_sha1 = sha1_hex(text.as_bytes());
    let meta: Option<EulaMeta> = fs::read_to_string(cache_dir(config).join(META_FILE))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok());
    let (source_url, fetched_at, sha1) = match meta {
        Some(meta) if meta.sha1.is_empty() || meta.sha1 == computed_sha1 => {
            (meta.url, meta.fetched_at, computed_sha1)
        }
        // 缓存文本和元数据不一致：宁可重新抓取，也不能展示来路不明的法务文本。
        Some(_) => return None,
        None => (EULA_URL.to_string(), 0, computed_sha1),
    };
    Some(EulaDocument {
        text,
        source_url,
        fetched_at,
        sha1,
        warning: None,
    })
}

/// 优先在线抓取；网络失败时退回本地缓存。
pub async fn load_or_fetch(config: &AppConfig) -> Result<EulaDocument, String> {
    match fetch(config).await {
        Ok(doc) => Ok(doc),
        Err(error) => match load_cached(config) {
            Some(mut doc) => {
                doc.warning = Some(format!("在线获取 EULA 失败，已使用本地缓存：{error}"));
                Ok(doc)
            }
            None => Err(error),
        },
    }
}

/// 启动时检查协议是否发生实质变化。
///
/// 先取旧缓存哈希（用于迁移只有 `eula.txt` 的旧用户），再在线抓取当前
/// 正文。网络失败直接返回错误，由 GUI 决定是否放行已接受版本；绝不能
/// 把“抓不到”误判成“协议未更新”。
pub async fn check_for_update(config: &AppConfig) -> Result<EulaUpdate, String> {
    if !is_accepted(config) {
        return Ok(EulaUpdate::NotAccepted);
    }
    let legacy_cached_sha1 = load_cached(config).map(|doc| doc.sha1);
    let current = fetch(config).await?;
    match accepted_sha1(config) {
        Some(accepted) if accepted == current.sha1 => Ok(EulaUpdate::Unchanged),
        Some(_) => Ok(EulaUpdate::Changed(current)),
        None => {
            // 旧版只有 eula.txt：如果旧缓存还在，就以旧缓存作为“曾经看过的
            // 版本”；没有缓存则只能把当前版本作为迁移基线。
            let baseline = legacy_cached_sha1.unwrap_or_else(|| current.sha1.clone());
            record_acceptance(config, &baseline)?;
            if baseline == current.sha1 {
                Ok(EulaUpdate::Unchanged)
            } else {
                Ok(EulaUpdate::Changed(current))
            }
        }
    }
}

fn record_acceptance(config: &AppConfig, sha1: &str) -> Result<(), String> {
    let dir = cache_dir(config);
    fs::create_dir_all(&dir)
        .map_err(|e| format!("创建 EULA 元数据目录失败 {}: {e}", dir.display()))?;
    let acceptance = EulaAcceptance {
        sha1: sha1.to_string(),
        source_url: EULA_URL.to_string(),
        accepted_at: unix_now(),
    };
    let json = serde_json::to_vec_pretty(&acceptance)
        .map_err(|e| format!("序列化 EULA 接受版本失败: {e}"))?;
    write_atomic(&acceptance_path(config), &json)
        .map_err(|e| format!("写入 EULA 接受版本失败: {e}"))
}

/// 抓取并解析官方 EULA。成功时才覆盖缓存。
pub async fn fetch(config: &AppConfig) -> Result<EulaDocument, String> {
    let client = net::client_http1_with_user_agent(EULA_USER_AGENT)?;
    let html = tokio::time::timeout(
        Duration::from_secs(25),
        net::get_text_once(&client, EULA_URL),
    )
    .await
    .map_err(|_| "获取 EULA 超时（25 秒）".to_string())??;
    let text = parse_eula_html(&html)?;
    let fetched_at = unix_now();
    let sha1 = sha1_hex(text.as_bytes());
    let doc = EulaDocument {
        text,
        source_url: EULA_URL.to_string(),
        fetched_at,
        sha1: sha1.clone(),
        warning: None,
    };

    let dir = cache_dir(config);
    fs::create_dir_all(&dir)
        .map_err(|e| format!("创建 EULA 缓存目录失败 {}: {e}", dir.display()))?;
    write_atomic(&dir.join(HTML_FILE), html.as_bytes())
        .map_err(|e| format!("写入 EULA 原始 HTML 失败: {e}"))?;
    write_atomic(&dir.join(TEXT_FILE), doc.text.as_bytes())
        .map_err(|e| format!("写入 EULA 文本失败: {e}"))?;
    let meta = EulaMeta {
        url: doc.source_url.clone(),
        fetched_at,
        sha1,
    };
    let meta =
        serde_json::to_vec_pretty(&meta).map_err(|e| format!("序列化 EULA 元数据失败: {e}"))?;
    write_atomic(&dir.join(META_FILE), &meta).map_err(|e| format!("写入 EULA 元数据失败: {e}"))?;

    Ok(doc)
}

/// 从官方页面 HTML 中提取 `#main-content` 的可见文本。
///
/// 只做“空格折叠 + 块级换行”，不删词、不改标点；链接文本按原位置保留。
/// 页面结构变化时宁可直接报错，也不能默默展示半截法务文本。
pub fn parse_eula_html(html: &str) -> Result<String, String> {
    let document = Html::parse_document(html);
    let selector = Selector::parse("#main-content").map_err(|e| format!("EULA 选择器无效: {e}"))?;
    let root = document
        .select(&selector)
        .next()
        .ok_or_else(|| "EULA 页面结构变化：找不到 #main-content".to_string())?;

    let mut raw = String::new();
    walk_element(root, &mut raw);
    let text = normalize_text(&raw);

    if !text.contains("Minecraft End(er)-User License Agreement (“EULA”)") {
        return Err("EULA 页面结构变化：缺少标题，拒绝展示不完整文本".to_string());
    }
    if !text.contains("COMPANY INFORMATION") {
        return Err(
            "EULA 页面结构变化：缺少 COMPANY INFORMATION 段，拒绝展示不完整文本".to_string(),
        );
    }
    if text.chars().count() < 5_000 {
        return Err(format!(
            "EULA 正文过短（{} 字符），疑似页面未完整返回",
            text.chars().count()
        ));
    }
    Ok(text)
}

fn walk_element(element: ElementRef<'_>, out: &mut String) {
    let name = element.value().name();
    if matches!(name, "script" | "style" | "noscript" | "template") {
        return;
    }
    if name == "br" {
        out.push('\n');
        return;
    }

    let block = matches!(
        name,
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "dd"
            | "div"
            | "dl"
            | "dt"
            | "figcaption"
            | "figure"
            | "footer"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "li"
            | "main"
            | "nav"
            | "ol"
            | "p"
            | "pre"
            | "section"
            | "table"
            | "tbody"
            | "td"
            | "tfoot"
            | "th"
            | "thead"
            | "tr"
            | "ul"
    );
    if block {
        ensure_newline(out);
    }

    for child in element.children() {
        match child.value() {
            Node::Text(text) => out.push_str(text),
            Node::Element(_) => {
                if let Some(child) = ElementRef::wrap(child) {
                    walk_element(child, out);
                }
            }
            _ => {}
        }
    }

    if block {
        ensure_newline(out);
    }
}

fn ensure_newline(out: &mut String) {
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
}

fn normalize_text(raw: &str) -> String {
    let normalized = raw.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines: Vec<String> = Vec::new();
    for line in normalized.split('\n') {
        let line = collapse_ascii_spaces(line);
        if line.is_empty() {
            if lines.last().is_some_and(|last| !last.is_empty()) {
                lines.push(String::new());
            }
        } else {
            lines.push(line);
        }
    }
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }

    let mut out = String::new();
    for line in lines {
        if line.is_empty() {
            if !out.is_empty() && !out.ends_with("\n\n") {
                out.push_str("\n\n");
            }
        } else {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&line);
        }
    }
    out.trim().to_string()
}

/// 只折叠 HTML 缩进产生的 ASCII 空格/制表符；不把 `&nbsp;`、全角空格等
/// 原文中的字符改掉。
fn collapse_ascii_spaces(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut pending_space = false;
    for ch in line.chars() {
        if ch == ' ' || ch == '\t' {
            pending_space = true;
        } else {
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            pending_space = false;
            out.push(ch);
        }
    }
    out
}

fn sha1_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("eula");
    let tmp = path.with_file_name(format!(".{file_name}.tmp-{}", std::process::id()));
    fs::write(&tmp, bytes).map_err(|e| format!("写入临时文件失败 {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("替换文件失败 {}: {e}", path.display())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r##"
        <html><body>
        <nav>不应出现的导航</nav>
        <div id="main-content" role="main">
          <h1><b>Minecraft End(er)-User License Agreement (“EULA”)<br /></b></h1>
          <h2><b>SUMMARY</b></h2>
          <p>This EULA is a legal agreement between you and us.</p>
          <ul><li>one</li><li>two</li></ul>
          <p>Hello&nbsp;world &amp; <a href="#">link text</a>.</p>
          <script>window.bad = true;</script>
          <h2>COMPANY INFORMATION</h2>
          <p>Mojang AB<br />Stockholm</p>
        </div>
        <footer>不应出现的页脚</footer>
        </body></html>
    "##;

    #[test]
    fn parses_main_content_only() {
        // 真实 EULA 远长于 5k 字符；给固定样例补一段正文，专门测试结构提取。
        let sample = SAMPLE.replace(
            "</div>\n        <footer>",
            &format!("<p>{}</p></div>\n        <footer>", "x".repeat(6_000)),
        );
        let text = parse_eula_html(&sample).unwrap();
        assert!(text.starts_with("Minecraft End(er)-User License Agreement (“EULA”)"));
        assert!(text.contains("This EULA is a legal agreement between you and us."));
        assert!(text.contains("Hello\u{a0}world & link text."));
        assert!(text.contains("one\ntwo"));
        assert!(text.contains("COMPANY INFORMATION"));
        assert!(text.contains("Mojang AB\nStockholm"));
        assert!(!text.contains("不应出现的导航"));
        assert!(!text.contains("不应出现的页脚"));
        assert!(!text.contains("window.bad"));
    }

    #[test]
    fn rejects_page_without_main_content() {
        assert!(parse_eula_html("<html><body><nav>nope</nav></body></html>").is_err());
    }

    #[test]
    fn marker_roundtrip() {
        let root = std::env::temp_dir().join(format!(
            "deepslate-eula-test-{}-{}",
            std::process::id(),
            unix_now()
        ));
        let config = AppConfig {
            root_dir: root.to_string_lossy().to_string(),
            ..AppConfig::default()
        };
        assert!(!is_accepted(&config));
        accept(&config).unwrap();
        assert!(is_accepted(&config));
        let marker = fs::read_to_string(marker_path(&config)).unwrap();
        assert_eq!(marker, EULA_MARKER_CONTENTS);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn acceptance_records_document_hash() {
        let root = std::env::temp_dir().join(format!(
            "deepslate-eula-accept-test-{}-{}",
            std::process::id(),
            unix_now()
        ));
        let config = AppConfig {
            root_dir: root.to_string_lossy().to_string(),
            ..AppConfig::default()
        };
        accept_document(&config, "abc123").unwrap();
        assert!(is_accepted(&config));
        assert_eq!(accepted_sha1(&config).as_deref(), Some("abc123"));

        // CLI 只表达“同意”，不知道版本；不能覆盖 GUI 已记录的版本绑定。
        accept(&config).unwrap();
        assert!(is_accepted(&config));
        assert_eq!(accepted_sha1(&config).as_deref(), Some("abc123"));
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    #[ignore = "requires network access to minecraft.net"]
    async fn fetches_live_official_eula() {
        let root = std::env::temp_dir().join(format!(
            "deepslate-eula-live-{}-{}",
            std::process::id(),
            unix_now()
        ));
        let config = AppConfig {
            root_dir: root.to_string_lossy().to_string(),
            ..AppConfig::default()
        };
        let doc = load_or_fetch(&config).await.unwrap();
        println!(
            "chars={} lines={} sha1={} fetched_at={}",
            doc.text.chars().count(),
            doc.text.lines().count(),
            doc.sha1,
            doc.fetched_at
        );
        println!("--- first ---\n{}", doc.text.lines().next().unwrap_or(""));
        println!("--- last ---\n{}", doc.text.lines().last().unwrap_or(""));
        assert!(doc.text.contains("Microsoft Corporation"));
        assert!(doc.text.contains("Mojang AB"));
        assert!(doc.text.len() > 5_000);
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    #[ignore = "requires network access to minecraft.net"]
    async fn live_check_detects_accepted_hash_mismatch() {
        let root = std::env::temp_dir().join(format!(
            "deepslate-eula-update-{}-{}",
            std::process::id(),
            unix_now()
        ));
        let config = AppConfig {
            root_dir: root.to_string_lossy().to_string(),
            ..AppConfig::default()
        };
        accept_document(&config, "definitely-an-old-eula-hash").unwrap();
        match check_for_update(&config).await.unwrap() {
            EulaUpdate::Changed(doc) => {
                assert!(doc
                    .text
                    .contains("Minecraft End(er)-User License Agreement"));
                assert_ne!(doc.sha1, "definitely-an-old-eula-hash");
            }
            other => panic!("expected Changed, got {other:?}"),
        }
        let _ = fs::remove_dir_all(root);
    }
}
