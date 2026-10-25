use serde::{Deserialize, Serialize};

// ── Minecraft News API（minecraft.net 官方搜索接口 + 文章页抓取） ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinecraftNewsResponse {
    pub items: Vec<MinecraftNewsItem>,
    pub total: u64,
    pub page: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinecraftNewsItem {
    pub title: String,
    pub description: String,
    pub url: String,
    pub date: String,
    pub image_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinecraftArticleDetail {
    pub title: String,
    pub description: String,
    pub author: String,
    pub date: String,
    pub image_url: Option<String>,
    pub url: String,
    pub images: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct MinecraftSearchResponse {
    result: MinecraftSearchResult,
}

#[derive(Debug, Deserialize)]
struct MinecraftSearchResult {
    results: Vec<MinecraftArticleItem>,
    #[serde(default)]
    #[serde(rename = "numFound")]
    num_found: u64,
    #[serde(default)]
    page: u64,
}

#[derive(Debug, Deserialize)]
struct MinecraftArticleItem {
    title: String,
    #[serde(default)]
    description: String,
    url: String,
    time: i64,
    #[serde(default)]
    #[serde(rename = "image")]
    image_url: Option<String>,
}

fn decode_html_entities(s: &str) -> String {
    s.replace("&#39;", "'")
     .replace("&#x27;", "'")
     .replace("&apos;", "'")
     .replace("&amp;", "&")
     .replace("&lt;", "<")
     .replace("&gt;", ">")
     .replace("&quot;", "\"")
     .replace("&#x2F;", "/")
}

/// Fetches Minecraft news from the official minecraft.net search API.
pub async fn fetch_minecraft_news(page: Option<u64>) -> Result<MinecraftNewsResponse, String> {
    let page = page.unwrap_or(1);
    let client = reqwest::Client::builder()
        .user_agent("Obsidian-Launcher/1.0")
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let api_url = format!(
        "https://net-secondary.web.minecraft-services.net/api/v1.0/zh-cn/search?pageSize=24&sortType=Recent&category=News&newsOnly=true&geography=CN&page={}",
        page
    );
    let resp = client.get(&api_url).send().await
        .map_err(|e| format!("Failed to fetch news: {}", e))?;
    let json = resp.text().await
        .map_err(|e| format!("Failed to read response: {}", e))?;

    let data: MinecraftSearchResponse = serde_json::from_str(&json)
        .map_err(|e| format!("Failed to parse news: {}", e))?;

    let items: Vec<MinecraftNewsItem> = data.result.results.into_iter().filter_map(|a| {
        let title = decode_html_entities(a.title.trim());
        if title.is_empty() { return None; }
        let date = if a.time > 0 { timestamp_to_date(a.time) } else { String::new() };
        let desc = if a.description.trim().is_empty() {
            "阅读更多 →".to_string()
        } else {
            decode_html_entities(&a.description)
        };
        Some(MinecraftNewsItem { title, description: desc, url: a.url, date, image_url: a.image_url })
    }).collect();

    Ok(MinecraftNewsResponse {
        items,
        total: data.result.num_found,
        page: data.result.page,
    })
}

/// Fetches article detail (JSON-LD metadata) from a minecraft.net article page.
pub async fn fetch_article_detail(url: String) -> Result<MinecraftArticleDetail, String> {
    let client = reqwest::Client::builder()
        .user_agent("Obsidian-Launcher/1.0")
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let resp = client.get(&url).send().await
        .map_err(|e| format!("Failed to fetch article: {}", e))?;
    let html = resp.text().await
        .map_err(|e| format!("Failed to read response: {}", e))?;

    // Parse JSON-LD with serde for reliable metadata extraction
    let mut author = String::new();
    let mut title_ld = String::new();
    let mut date_ld = String::new();
    let mut desc_og = String::new();
    let mut image_og: Option<String> = None;

    if let Some(ld_start) = html.find(r#"type="application/ld+json""#) {
        let after_tag = &html[ld_start..];
        if let Some(script_end) = after_tag.find("</script>") {
            if let Some(json_start) = after_tag[..script_end].rfind('>') {
                let json_str = &after_tag[json_start+1..script_end];
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
                    if let Some(a) = val.get("author") {
                        author = a.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    }
                    if let Some(h) = val.get("headline").and_then(|v| v.as_str()) {
                        title_ld = h.to_string();
                    }
                    if let Some(d) = val.get("datePublished").and_then(|v| v.as_str()) {
                        date_ld = if d.len() > 10 { d[..10].to_string() } else { d.to_string() };
                    }
                }
            }
        }
    }

    // OG description
    if let Some(start) = html.find(r#"property="og:description" content=""#) {
        let slice = &html[start + 36..];
        if let Some(end) = slice.find('"') {
            desc_og = decode_html_entities(&slice[..end]);
        }
    }

    // OG image
    if let Some(start) = html.find(r#"property="og:image" content=""#) {
        let slice = &html[start + 31..];
        if let Some(end) = slice.find('"') {
            image_og = Some(decode_html_entities(&slice[..end]));
        }
    }

    // Extract all article body images (filter icons/avatars/logos)
    let mut images: Vec<String> = Vec::new();
    if let Some(art_start) = html.find(r#"class="article-text""#) {
        let footer = html[art_start..].find("<footer ").unwrap_or(html.len() - art_start);
        let article_area = &html[art_start..art_start + footer];
        let mut pos = 0;
        while let Some(img) = article_area[pos..].find("<img ") {
            let abs = pos + img;
            if let Some(end) = article_area[abs..].find('>') {
                let tag = &article_area[abs..abs + end];
                if let Some(s) = tag.find("src=\"") {
                    let ss = &tag[s + 5..];
                    if let Some(se) = ss.find('"') {
                        let src = &ss[..se];
                        let lower = src.to_lowercase();
                        // Filter out icons, avatars, logos, and small assets
                        if !lower.contains("icon") && !lower.contains("avatar")
                           && !lower.contains("logo") && !lower.contains("svg")
                           && !lower.contains("cursors") {
                            let full = if src.starts_with("http") { src.to_string() }
                                else if src.starts_with("//") { format!("https:{}", src) }
                                else { format!("https://www.minecraft.net{}", src) };
                            if !images.contains(&full) {
                                images.push(full);
                            }
                        }
                    }
                }
                pos = abs + end + 1;
            } else { break; }
        }
    }

    // Extract article body from SSR <article-text> with boundary check
    let mut body = String::new();
    if let Some(art_start) = html.find(r#"class="article-text""#) {
        let slice = &html[art_start..];
        // Find boundary: stop before footer or related-articles section
        let end_pos = [
            "<footer ",
            r#"class="article-sidebar""#,
            r#"class="related-articles""#,
        ].iter().filter_map(|m| slice.find(m)).min().unwrap_or(slice.len());
        let article_slice = &slice[..end_pos];

        // Extract <p>, <h2>, <h3>, <li> in document order
        let tags: &[(&str, &str, &str)] = &[
            ("<h2>", "</h2>", "## "),
            ("<h3>", "</h3>", "### "),
            ("<li>", "</li>", "- "),
            ("<p>",  "</p>",  ""),
        ];
        let mut pos = 0;
        while pos < article_slice.len() {
            // Find the earliest tag at or after pos
            let mut earliest: Option<(usize, &str, &str, &str)> = None;
            for (open, close, prefix) in tags {
                if let Some(start) = article_slice[pos..].find(open) {
                    let score = pos + start;
                    if earliest.map_or(true, |e| score < e.0) {
                        earliest = Some((score, open, close, prefix));
                    }
                }
            }
            if let Some((start, open, close, prefix)) = earliest {
                let after_open = start + open.len();
                if let Some(close_at) = article_slice[after_open..].find(close) {
                    let content_end = after_open + close_at;
                    let text = strip_html(&article_slice[after_open..content_end]);
                    if !text.is_empty() {
                        if !body.is_empty() { body.push('\n'); }
                        body.push_str(prefix);
                        body.push_str(&text);
                    }
                    pos = content_end + close.len();
                } else { break; }
            } else { break; }
        }
    }
    // Use extracted body, fallback to OG description
    let description = if !body.is_empty() {
        decode_html_entities(&body)
    } else {
        desc_og
    };

    Ok(MinecraftArticleDetail {
        title: decode_html_entities(&title_ld),
        description,
        author: decode_html_entities(&author),
        date: date_ld,
        image_url: image_og,
        url,
        images,
    })
}

fn strip_html(input: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in input.chars() {
        if c == '<' { in_tag = true; continue; }
        if c == '>' { in_tag = false; continue; }
        if !in_tag { out.push(c); }
    }
    decode_html_entities(&out.trim().to_string())
}

fn timestamp_to_date(ts: i64) -> String {
    // Howard Hinnant's civil_from_days — correct for all Gregorian dates
    let z = ts / 86400 + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02}", y, m, d)
}
