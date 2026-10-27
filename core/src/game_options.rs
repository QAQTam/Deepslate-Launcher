//! `options.txt` 默认值注入。
//!
//! 目标：首次进入游戏就是中文，并且跳过辅助功能引导 / 教程提示；
//! **只补缺失的键**，不覆盖玩家在游戏内已经做出的选择。
//!
//! 26.3 的 `Options` 字段与键名（从客户端 jar 解析确认）：
//! `lang`、`onboardAccessibility`、`skipMultiplayerWarning`、`tutorialStep`、`narrator`。

use std::path::Path;

#[derive(Debug, Clone)]
pub struct GameDefaults {
    pub language: String,
    pub skip_accessibility_onboarding: bool,
    pub skip_multiplayer_warning: bool,
    pub skip_tutorial: bool,
    pub narrator_off: bool,
}

impl Default for GameDefaults {
    fn default() -> Self {
        Self {
            language: "zh_cn".to_string(),
            skip_accessibility_onboarding: true,
            skip_multiplayer_warning: true,
            skip_tutorial: true,
            narrator_off: true,
        }
    }
}

impl GameDefaults {
    /// 要写入的 (key, value) 列表；只有缺失时才会追加。
    fn entries(&self) -> Vec<(&'static str, String)> {
        let mut out = Vec::new();
        if !self.language.trim().is_empty() {
            out.push(("lang", self.language.clone()));
        }
        if self.skip_accessibility_onboarding {
            out.push(("onboardAccessibility", "false".to_string()));
        }
        if self.skip_multiplayer_warning {
            out.push(("skipMultiplayerWarning", "true".to_string()));
        }
        if self.skip_tutorial {
            out.push(("tutorialStep", "none".to_string()));
        }
        if self.narrator_off {
            out.push(("narrator", "0".to_string()));
        }
        out
    }
}

/// 确保实例目录下的 `options.txt` 含默认键。返回是否修改了文件。
pub fn ensure_defaults(instance_dir: &Path, defaults: &GameDefaults) -> Result<bool, String> {
    let path = instance_dir.join("options.txt");
    let marker = instance_dir.join(".deepslate").join("game_defaults_v1");
    let existed = path.exists();
    let content = if existed {
        std::fs::read_to_string(&path).map_err(|e| format!("读取 options.txt 失败: {e}"))?
    } else {
        String::new()
    };

    let mut lines: Vec<String> = content
        .lines()
        .map(|line| line.trim_end_matches('\r').to_string())
        .collect();
    let mut present: std::collections::HashSet<String> = std::collections::HashSet::new();
    for line in &lines {
        if let Some((key, _)) = line.split_once(':') {
            present.insert(key.to_string());
        }
    }

    let mut changed = false;
    for (key, value) in defaults.entries() {
        if !present.contains(key) {
            lines.push(format!("{key}:{value}"));
            changed = true;
        }
    }

    if !changed && existed {
        return Ok(false);
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建实例目录失败 {}: {e}", parent.display()))?;
    }
    let mut text = lines.join("\n");
    text.push('\n');
    let tmp = {
        let mut name = path.as_os_str().to_os_string();
        name.push(".part");
        std::path::PathBuf::from(name)
    };
    std::fs::write(&tmp, text.as_bytes())
        .map_err(|e| format!("写入 options.txt 失败 {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path)
        .map_err(|e| format!("替换 options.txt 失败 {}: {e}", path.display()))?;

    if let Some(parent) = marker.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&marker, b"1");
    Ok(true)
}

/// Explicitly apply a language selected in the launcher. Unlike first-run defaults,
/// this intentionally replaces the existing `lang` value while preserving all other options.
pub fn set_language(instance_dir: &Path, language: &str) -> Result<bool, String> {
    let bytes = language.as_bytes();
    if bytes.len() != 5
        || bytes[0..2].iter().any(|b| !b.is_ascii_lowercase())
        || bytes[2] != b'_'
        || bytes[3..5].iter().any(|b| !b.is_ascii_lowercase())
    {
        return Err("语言代码无效".to_string());
    }

    let path = instance_dir.join("options.txt");
    let content = if path.exists() {
        std::fs::read_to_string(&path).map_err(|e| format!("读取 options.txt 失败: {e}"))?
    } else {
        String::new()
    };
    let mut lines = Vec::new();
    let mut found = false;
    let mut changed = false;
    for line in content.lines() {
        if line.starts_with("lang:") {
            if !found {
                let replacement = format!("lang:{language}");
                changed |= line != replacement;
                lines.push(replacement);
                found = true;
            } else {
                changed = true;
            }
        } else {
            lines.push(line.trim_end_matches('\r').to_string());
        }
    }
    if !found {
        lines.push(format!("lang:{language}"));
        changed = true;
    }
    if !changed {
        return Ok(false);
    }
    std::fs::create_dir_all(instance_dir)
        .map_err(|e| format!("创建实例目录失败 {}: {e}", instance_dir.display()))?;
    let mut text = lines.join("\n");
    text.push('\n');
    let mut temp_name = path.as_os_str().to_os_string();
    temp_name.push(".part");
    let temp = std::path::PathBuf::from(temp_name);
    std::fs::write(&temp, text.as_bytes())
        .map_err(|e| format!("写入 options.txt 失败 {}: {e}", temp.display()))?;
    std::fs::rename(&temp, &path)
        .map_err(|e| format!("替换 options.txt 失败 {}: {e}", path.display()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "deepslate-options-test-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn first_takeover_preserves_existing_accessibility_and_language() {
        let dir = temp_dir("preserve-first");
        std::fs::write(
            dir.join("options.txt"),
            "lang:en_us\nnarrator:3\nonboardAccessibility:true\n",
        )
        .unwrap();
        ensure_defaults(&dir, &GameDefaults::default()).unwrap();
        let text = read(&dir.join("options.txt"));
        assert!(text.contains("lang:en_us"));
        assert!(text.contains("narrator:3"));
        assert!(text.contains("onboardAccessibility:true"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn creates_chinese_defaults_for_new_instance() {
        let dir = temp_dir("new");
        assert!(ensure_defaults(&dir, &GameDefaults::default()).unwrap());
        let text = read(&dir.join("options.txt"));
        assert!(text.contains("lang:zh_cn"));
        assert!(text.contains("onboardAccessibility:false"));
        assert!(text.contains("skipMultiplayerWarning:true"));
        assert!(text.contains("tutorialStep:none"));
        assert!(text.contains("narrator:0"));
        assert!(text.ends_with('\n'));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn first_apply_preserves_player_choices() {
        let dir = temp_dir("existing");
        std::fs::write(
            dir.join("options.txt"),
            "lang:en_us\nonboardAccessibility:true\nfov:0.5\n",
        )
        .unwrap();

        // 已有设置保留，只补缺失项。
        assert!(ensure_defaults(&dir, &GameDefaults::default()).unwrap());
        let text = read(&dir.join("options.txt"));
        assert!(text.contains("lang:en_us"));
        assert!(text.contains("onboardAccessibility:true"));
        assert!(text.contains("fov:0.5"));
        assert!(text.contains("tutorialStep:none"));

        // 玩家随后在游戏内改回英文：不应被再次覆盖
        let text = read(&dir.join("options.txt"));
        std::fs::write(dir.join("options.txt"), text).unwrap();
        assert!(!ensure_defaults(&dir, &GameDefaults::default()).unwrap());
        assert!(read(&dir.join("options.txt")).contains("lang:en_us"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn second_run_is_noop() {
        let dir = temp_dir("noop");
        assert!(ensure_defaults(&dir, &GameDefaults::default()).unwrap());
        assert!(!ensure_defaults(&dir, &GameDefaults::default()).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
