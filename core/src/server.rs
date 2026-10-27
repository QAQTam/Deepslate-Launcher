//! 本地服务端安装与进程管理（原版 server.jar）。
//!
//! - 从 `versions/<id>/<id>.json` 沿 `inheritsFrom` 找到 `downloads.server`
//! - 下载到 `<root>/servers/<selected-version>/server.jar`
//! - 写 `eula.txt` / 更新 `server.properties`（保留其它键）
//! - 启动 `java -Xmx<mem>M -jar server.jar nogui`，日志转发为 `CoreEvent::ServerLog`
//! - 检测 `Done (` 后发送 `ServerState::Ready`，GUI 可据此自动加入
//!
//! Fabric 服务端需要 fabric-server-launch，当前先使用基础原版服务端；
//! 客户端装 Fabric mod 后连接原版服务端也能正常游玩（服务端不加载 mod）。

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{LazyLock, Mutex};

use tokio::fs;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::mpsc;

use crate::config::AppConfig;
use crate::download::{self, DownloadTask};
use crate::events::{CoreEvent, ServerState};

static SERVER_PID: Mutex<Option<u32>> = Mutex::new(None);
static SERVER_STDIN: LazyLock<tokio::sync::Mutex<Option<ChildStdin>>> =
    LazyLock::new(|| tokio::sync::Mutex::new(None));

#[derive(Debug, Clone)]
pub struct ServerOptions {
    /// 当前选中的版本（可以是 Fabric 版本，服务端会沿继承链取原版 jar）
    pub version: String,
    pub versions_dir: PathBuf,
    pub server_dir: PathBuf,
    pub java_home: PathBuf,
    pub memory_mb: u32,
    pub port: u16,
    pub online_mode: bool,
    pub accept_eula: bool,
    /// 非空表示使用 Fabric 服务端启动器（KnotServer），否则为原版 `-jar`
    pub fabric_libraries: Vec<PathBuf>,
}

impl ServerOptions {
    pub fn new(
        config: &AppConfig,
        version: &str,
        java_home: PathBuf,
        memory_mb: u32,
        port: u16,
        online_mode: bool,
        accept_eula: bool,
    ) -> Self {
        Self {
            version: version.to_string(),
            versions_dir: crate::config::resolve_game_dir(config).join("versions"),
            server_dir: server_dir(config, version),
            java_home,
            memory_mb,
            port,
            online_mode,
            accept_eula,
            fabric_libraries: detect_fabric_libraries(
                &crate::config::resolve_game_dir(config).join("versions"),
                &crate::config::resolve_game_dir(config).join("libraries"),
                version,
            ),
        }
    }
}

/// 读取本地版本 JSON：若它是 Fabric 客户端/服务端 profile，返回启动 Fabric
/// 服务端所需的库路径（asm / sponge-mixin / loader / intermediary）。
fn detect_fabric_libraries(
    versions_dir: &Path,
    libraries_dir: &Path,
    version: &str,
) -> Vec<PathBuf> {
    let path = versions_dir.join(version).join(format!("{version}.json"));
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_str::<Value>(&text) else {
        return Vec::new();
    };
    let main_class = json.get("mainClass").and_then(Value::as_str).unwrap_or("");
    let libs = json.get("libraries").and_then(Value::as_array);
    let is_fabric = main_class.contains("fabric")
        || libs
            .map(|libs| {
                libs.iter().any(|lib| {
                    lib.get("name")
                        .and_then(Value::as_str)
                        .map(|n| n.starts_with("net.fabricmc:fabric-loader"))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false);
    if !is_fabric {
        return Vec::new();
    }
    let mut paths = Vec::new();
    if let Some(libs) = libs {
        for lib in libs {
            let Some(name) = lib.get("name").and_then(Value::as_str) else {
                continue;
            };
            if let Some(rel) = crate::launch::maven_artifact_path(name) {
                paths.push(libraries_dir.join(rel));
            }
        }
    }
    paths
}

pub fn server_dir(config: &AppConfig, version: &str) -> PathBuf {
    PathBuf::from(&config.root_dir)
        .join("servers")
        .join(version)
}

pub fn is_running() -> bool {
    SERVER_PID.lock().map(|p| p.is_some()).unwrap_or(false)
}

/// 沿继承链找 `downloads.server`，返回 (声明它的版本 id, url, sha1)。
fn find_server_download(
    versions_dir: &Path,
    version: &str,
) -> Result<(String, String, String), String> {
    let mut current = version.to_string();
    let mut seen = std::collections::HashSet::new();
    for _ in 0..16 {
        crate::launch::validate_version_id(&current)?;
        if !seen.insert(current.clone()) {
            return Err("版本继承链出现循环".to_string());
        }
        let path = versions_dir.join(&current).join(format!("{current}.json"));
        let text = std::fs::read_to_string(&path).map_err(|e| {
            format!(
                "读取版本 JSON 失败 {}: {e}（请先下载该版本）",
                path.display()
            )
        })?;
        let json: Value = serde_json::from_str(&text)
            .map_err(|e| format!("解析版本 JSON 失败 {}: {e}", path.display()))?;
        if let Some(server) = json
            .get("downloads")
            .and_then(|d| d.get("server"))
            .and_then(|s| s.as_object())
        {
            let url = server
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| "服务端下载缺少 url".to_string())?;
            let sha1 = server.get("sha1").and_then(Value::as_str).unwrap_or("");
            return Ok((current, url.to_string(), sha1.to_string()));
        }
        match json.get("inheritsFrom").and_then(Value::as_str) {
            Some(parent) => current = parent.to_string(),
            None => {
                return Err(format!(
                    "版本 {version} 的继承链里没有服务端下载信息（旧版本可能只提供客户端）"
                ))
            }
        }
    }
    Err("版本继承链过深".to_string())
}

/// 下载/校验服务端 jar，并写入 EULA 与基础 server.properties。
pub async fn install(
    opts: &ServerOptions,
    events: &mpsc::Sender<CoreEvent>,
) -> Result<PathBuf, String> {
    if !opts.accept_eula {
        return Err("需要先同意 Minecraft EULA 才能安装/启动服务端".to_string());
    }
    let jar = ensure_server_jar(opts, events).await?;
    write_eula(&opts.server_dir).await?;
    write_properties(&opts.server_dir, opts.port, opts.online_mode).await?;
    Ok(jar)
}

/// 只确保 server.jar 存在（不碰 server.properties / eula），供开服与建世界共用。
async fn ensure_server_jar(
    opts: &ServerOptions,
    events: &mpsc::Sender<CoreEvent>,
) -> Result<PathBuf, String> {
    if opts.server_dir.as_os_str().is_empty() {
        return Err("服务端目录为空".to_string());
    }
    fs::create_dir_all(&opts.server_dir)
        .await
        .map_err(|e| format!("创建服务端目录失败 {}: {e}", opts.server_dir.display()))?;

    let (base_id, url, sha1) = find_server_download(&opts.versions_dir, &opts.version)?;
    let jar = opts.server_dir.join("server.jar");
    println!("服务端来源版本: {base_id}");
    download::download_all(
        vec![DownloadTask {
            url,
            path: jar.clone(),
            sha1: (!sha1.is_empty()).then_some(sha1),
        }],
        1,
        events.clone(),
        "正在下载服务端...",
    )
    .await?;
    Ok(jar)
}

async fn write_eula(dir: &Path) -> Result<(), String> {
    fs::write(
        dir.join("eula.txt"),
        crate::eula::EULA_MARKER_CONTENTS.as_bytes(),
    )
    .await
    .map_err(|e| format!("写入 eula.txt 失败: {e}"))
}

async fn write_properties(dir: &Path, port: u16, online_mode: bool) -> Result<(), String> {
    let path = dir.join("server.properties");
    let existing = fs::read_to_string(&path).await.unwrap_or_default();
    let defaults = [
        ("level-name", "world".to_string()),
        ("motd", "Deepslate Server".to_string()),
        ("max-players", "10".to_string()),
        ("view-distance", "8".to_string()),
        ("simulation-distance", "8".to_string()),
        ("enable-command-block", "false".to_string()),
        ("spawn-protection", "0".to_string()),
        ("server-port", port.to_string()),
        ("server-ip", "127.0.0.1".to_string()),
        ("online-mode", online_mode.to_string()),
    ];

    let mut lines: Vec<String> = existing.lines().map(str::to_string).collect();
    for (key, value) in defaults {
        // 此界面承诺本地服务端；明确绑定 loopback，避免旧配置暴露到局域网。
        let forced = matches!(key, "server-port" | "server-ip" | "online-mode");
        let mut found = false;
        for line in lines.iter_mut() {
            if let Some((k, _)) = line.split_once('=') {
                if k.trim() == key {
                    if forced {
                        *line = format!("{key}={value}");
                    }
                    found = true;
                    break;
                }
            }
        }
        if !found {
            lines.push(format!("{key}={value}"));
        }
    }
    let mut text = lines.join("\n");
    text.push('\n');
    fs::write(&path, text)
        .await
        .map_err(|e| format!("写入 server.properties 失败: {e}"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldType {
    Normal,
    Flat,
    LargeBiomes,
    Amplified,
}

impl WorldType {
    fn level_type(self) -> &'static str {
        match self {
            WorldType::Normal => "minecraft:normal",
            WorldType::Flat => "minecraft:flat",
            WorldType::LargeBiomes => "minecraft:large_biomes",
            WorldType::Amplified => "minecraft:amplified",
        }
    }

    fn generator_settings(self) -> &'static str {
        match self {
            WorldType::Flat => {
                r#"{"layers":[{"block":"minecraft:bedrock","height":1},{"block":"minecraft:dirt","height":2},{"block":"minecraft:grass_block","height":1}],"biome":"minecraft:plains"}"#
            }
            _ => "{}",
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorldCreationOptions {
    pub server: ServerOptions,
    pub saves_dir: PathBuf,
    pub name: String,
    pub seed: String,
    pub world_type: WorldType,
    /// survival / creative / adventure / spectator
    pub game_mode: String,
    pub cheats: bool,
}

/// 用后台原版服务端生成一个世界，移动到 saves_dir，并写入显示名/作弊/模式。
/// 返回新存档的文件夹 id（可直接用于 quickPlaySingleplayer）。
pub async fn create_world(
    opts: &WorldCreationOptions,
    events: &mpsc::Sender<CoreEvent>,
) -> Result<String, String> {
    create_world_with_job(opts, events, None).await
}

/// 与 [`create_world`] 相同，但把生成进程 PID 写入 `job_file`，
/// 供启动器崩溃后的陈旧任务清理使用。
pub async fn create_world_with_job(
    opts: &WorldCreationOptions,
    events: &mpsc::Sender<CoreEvent>,
    job_file: Option<&Path>,
) -> Result<String, String> {
    if !opts.server.accept_eula {
        return Err("需要先同意 Minecraft EULA 才能生成世界".to_string());
    }
    let name = opts.name.trim();
    if name.is_empty() {
        return Err("世界名称不能为空".to_string());
    }
    if name.chars().count() > 64 {
        return Err("世界名称过长（最多 64 字符）".to_string());
    }
    fs::create_dir_all(&opts.saves_dir)
        .await
        .map_err(|e| format!("创建存档目录失败: {e}"))?;

    let jar = ensure_server_jar(&opts.server, events).await?;
    let temp = worldgen_temp_path(&opts.server.server_dir);
    fs::create_dir(&temp)
        .await
        .map_err(|e| format!("创建临时生成目录失败: {e}"))?;

    if let Err(e) = generate_in_temp(opts, &jar, &temp, events, job_file).await {
        let _ = fs::remove_dir_all(&temp).await;
        return Err(e);
    }

    let source = temp.join("world");
    if let Err(e) = crate::saves::validate_world_dir(&source) {
        let _ = fs::remove_dir_all(&temp).await;
        return Err(format!("服务端生成的世界未通过校验: {e}"));
    }

    let mut id = "new-world".to_string();
    let mut counter = 2;
    while opts.saves_dir.join(&id).exists() {
        id = format!("new-world-{counter}");
        counter += 1;
    }
    let target = opts.saves_dir.join(&id);
    if std::fs::rename(&source, &target).is_err() {
        crate::saves::copy_dir(&source, &target)?;
        let _ = std::fs::remove_dir_all(&source);
    }
    let _ = std::fs::remove_file(target.join("session.lock"));
    let _ = fs::remove_dir_all(&temp).await;

    if let Err(e) = apply_world_metadata(&target, name, &opts.game_mode, opts.cheats) {
        let _ = fs::remove_dir_all(&target);
        return Err(e);
    }

    // 在 create_world 返回前先落一个最小完成标记，缩小“世界已移动、launcher
    // 还没写完整标记就崩溃”的窗口。worldgen 随后会覆盖成带 generator 的版本。
    if let Some(job_file) = job_file {
        if let Some(parent) = job_file.parent() {
            let marker = serde_json::json!({ "world": &id });
            if let Ok(bytes) = serde_json::to_vec_pretty(&marker) {
                let _ = std::fs::write(parent.join(".complete.json"), bytes);
            }
        }
    }
    Ok(id)
}

fn worldgen_temp_path(root: &Path) -> PathBuf {
    root.join(format!(".worldgen-{}", uuid::Uuid::new_v4()))
}

async fn generate_in_temp(
    opts: &WorldCreationOptions,
    jar: &Path,
    temp: &Path,
    events: &mpsc::Sender<CoreEvent>,
    job_file: Option<&Path>,
) -> Result<(), String> {
    fs::write(temp.join("eula.txt"), b"eula=true\n")
        .await
        .map_err(|e| format!("写入 eula.txt 失败: {e}"))?;
    let mut props = String::new();
    props.push_str("level-name=world\n");
    props.push_str("server-port=0\n");
    props.push_str("online-mode=false\n");
    props.push_str("max-players=1\n");
    props.push_str("view-distance=2\n");
    props.push_str("simulation-distance=2\n");
    props.push_str("spawn-protection=0\n");
    props.push_str("enable-status=false\n");
    props.push_str("enable-command-block=false\n");
    props.push_str("allow-nether=true\n");
    props.push_str("sync-chunk-writes=false\n");
    props.push_str("motd=Deepslate World Generation\n");
    props.push_str(&format!("level-type={}\n", opts.world_type.level_type()));
    props.push_str(&format!(
        "generator-settings={}\n",
        opts.world_type.generator_settings()
    ));
    props.push_str(&format!(
        "gamemode={}\n",
        if opts.game_mode.is_empty() {
            "survival"
        } else {
            &opts.game_mode
        }
    ));
    if !opts.seed.trim().is_empty() {
        props.push_str(&format!("level-seed={}\n", opts.seed.trim()));
    }
    fs::write(temp.join("server.properties"), props)
        .await
        .map_err(|e| format!("写入 server.properties 失败: {e}"))?;

    let java =
        opts.server
            .java_home
            .join("bin")
            .join(if cfg!(windows) { "java.exe" } else { "java" });
    let memory = opts.server.memory_mb.clamp(1024, 65_536);
    let mut child = Command::new(&java)
        .arg(format!("-Xmx{memory}M"))
        .arg("-jar")
        .arg(jar)
        .arg("nogui")
        .current_dir(temp)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("启动世界生成服务端失败: {e}"))?;

    let pid = child
        .id()
        .ok_or_else(|| "无法获取世界生成服务端 PID".to_string())?;
    let local_marker = temp.join(".deepslate-worldgen.json");
    let marker_result = write_worldgen_marker(&local_marker, pid, temp).and_then(|()| {
        job_file
            .map(|path| write_worldgen_marker(path, pid, temp))
            .unwrap_or(Ok(()))
    });
    if let Err(e) = marker_result {
        let _ = child.kill().await;
        let _ = child.wait().await;
        return Err(format!("写入生成任务标记失败: {e}"));
    }

    let mut stdin = child.stdin.take();
    if let Some(stderr) = child.stderr.take() {
        let events = events.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = events
                    .send(CoreEvent::ServerLog {
                        is_stderr: true,
                        line,
                    })
                    .await;
            }
        });
    }
    let stdout = child.stdout.take().ok_or("无法读取服务端 stdout")?;
    let mut lines = BufReader::new(stdout).lines();
    let ready = tokio::time::timeout(std::time::Duration::from_secs(300), async {
        while let Ok(Some(line)) = lines.next_line().await {
            let done = line.contains("Done (");
            let _ = events
                .send(CoreEvent::ServerLog {
                    is_stderr: false,
                    line,
                })
                .await;
            if done {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false);

    if !ready {
        let _ = child.kill().await;
        let _ = child.wait().await;
        return Err("世界生成超时或服务端提前退出".to_string());
    }

    if let Some(stdin) = stdin.as_mut() {
        let _ = stdin.write_all(b"stop\n").await;
        let _ = stdin.flush().await;
    }
    drop(stdin);
    let stopped = tokio::time::timeout(std::time::Duration::from_secs(60), child.wait()).await;
    match stopped {
        Ok(Ok(_)) => Ok(()),
        _ => {
            let _ = child.kill().await;
            Err("停止世界生成服务端超时".to_string())
        }
    }
}

fn write_worldgen_marker(path: &Path, pid: u32, cwd: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建生成标记目录失败 {}: {e}", parent.display()))?;
    }
    let value = serde_json::json!({
        "pid": pid,
        "owner_pid": std::process::id(),
        "started_at": crate::saves::now_ms(),
        "cwd": cwd.to_string_lossy(),
    });
    std::fs::write(
        path,
        serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("写入生成标记失败 {}: {e}", path.display()))
}

/// 检查一个生成进程是否还活着。只用于清理带 PID 标记的陈旧任务。
pub fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        return std::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
    }
    #[cfg(windows)]
    {
        let Ok(output) = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output()
        else {
            return false;
        };
        let text = String::from_utf8_lossy(&output.stdout);
        return text.contains(&pid.to_string());
    }
    #[allow(unreachable_code)]
    false
}

/// 终止一个生成进程；Unix 先 TERM 再 KILL，Windows 使用 taskkill 树。
pub fn terminate_pid(pid: u32) {
    if pid == 0 || pid == std::process::id() {
        return;
    }
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let _ = std::process::Command::new("kill")
            .arg("-KILL")
            .arg(pid.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn apply_world_metadata(
    dir: &Path,
    name: &str,
    game_mode: &str,
    cheats: bool,
) -> Result<(), String> {
    let path = dir.join("level.dat");
    let mut nbt = crate::nbt::NbtFile::read_gzip(&path)?;
    let data = nbt
        .root
        .as_compound_mut()
        .and_then(|root| root.get_mut("Data"))
        .ok_or_else(|| "level.dat 缺少 Data".to_string())?;
    let data = data
        .as_compound_mut()
        .ok_or_else(|| "level.dat 的 Data 不是 Compound".to_string())?;
    data.insert(
        "LevelName".to_string(),
        crate::nbt::Tag::String(name.to_string()),
    );
    data.insert(
        "GameType".to_string(),
        crate::nbt::Tag::Int(game_mode_id(game_mode)),
    );
    data.insert(
        "allowCommands".to_string(),
        crate::nbt::Tag::Byte(if cheats { 1 } else { 0 }),
    );
    data.insert(
        "LastPlayed".to_string(),
        crate::nbt::Tag::Long(crate::saves::now_ms()),
    );
    if let Some(crate::nbt::Tag::Compound(settings)) = data.get_mut("difficulty_settings") {
        settings.insert("hardcore".to_string(), crate::nbt::Tag::Byte(0));
    }
    nbt.write_gzip(&path)
}

fn game_mode_id(mode: &str) -> i32 {
    match mode {
        "creative" => 1,
        "adventure" => 2,
        "spectator" => 3,
        _ => 0,
    }
}

/// 启动服务端；日志通过 `CoreEvent::ServerLog`、状态通过 `CoreEvent::ServerState` 上报。
pub async fn start(opts: ServerOptions, events: mpsc::Sender<CoreEvent>) -> Result<(), String> {
    if is_running() {
        return Err("服务端已经在运行".to_string());
    }
    if opts.port == 0 {
        return Err("端口不能为 0".to_string());
    }
    let memory = opts.memory_mb.clamp(512, 65_536);
    let jar = install(&opts, &events).await?;
    if !opts.fabric_libraries.is_empty() {
        fs::create_dir_all(opts.server_dir.join("mods"))
            .await
            .map_err(|e| format!("创建服务端 mods 目录失败: {e}"))?;
    }

    let java = opts
        .java_home
        .join("bin")
        .join(if cfg!(windows) { "java.exe" } else { "java" });
    if !java.exists() {
        return Err(format!("Java 不存在: {}", java.display()));
    }

    let _ = events
        .send(CoreEvent::ServerState(ServerState::Starting {
            version: opts.version.clone(),
        }))
        .await;

    let mut command = Command::new(&java);
    command.arg(format!("-Xmx{memory}M"));
    if !opts.fabric_libraries.is_empty() {
        let missing: Vec<&PathBuf> = opts
            .fabric_libraries
            .iter()
            .filter(|p| !p.is_file())
            .collect();
        if !missing.is_empty() {
            return Err(format!(
                "Fabric 服务端库缺失: {}（请重新安装该 Fabric 版本）",
                missing
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let classpath = std::env::join_paths(&opts.fabric_libraries)
            .map_err(|e| format!("构建 Fabric 服务端 classpath 失败: {e}"))?;
        let game_jar = std::fs::canonicalize(&jar).unwrap_or_else(|_| jar.clone());
        command
            .arg(format!("-Dfabric.gameJarPath={}", game_jar.display()))
            .arg("-cp")
            .arg(classpath)
            .arg("net.fabricmc.loader.impl.launch.server.FabricServerLauncher");
        println!("使用 Fabric 服务端启动器（KnotServer）");
    } else {
        command.arg("-jar").arg(&jar);
    }
    command
        .arg("nogui")
        .current_dir(&opts.server_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|e| format!("启动服务端失败 ({}): {e}", java.display()))?;

    let pid = child.id().ok_or("无法获取服务端进程 ID")?;
    if let Some(stdin) = child.stdin.take() {
        *SERVER_STDIN.lock().await = Some(stdin);
    }
    {
        let mut guard = SERVER_PID.lock().map_err(|e| e.to_string())?;
        *guard = Some(pid);
    }
    let _ = events
        .send(CoreEvent::ServerState(ServerState::Running { pid }))
        .await;

    // 日志转发 + Ready 检测
    if let Some(stdout) = child.stdout.take() {
        let events = events.clone();
        let port = opts.port;
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let ready = line.contains("Done (");
                let _ = events
                    .send(CoreEvent::ServerLog {
                        is_stderr: false,
                        line: line.clone(),
                    })
                    .await;
                if ready {
                    let _ = events
                        .send(CoreEvent::ServerState(ServerState::Ready { port }))
                        .await;
                }
            }
        });
    }
    if let Some(stderr) = child.stderr.take() {
        let events = events.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = events
                    .send(CoreEvent::ServerLog {
                        is_stderr: true,
                        line,
                    })
                    .await;
            }
        });
    }

    // 等待退出，清理状态
    let wait_events = events.clone();
    tokio::spawn(async move {
        let status = child.wait().await;
        let exit_code = status.map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);
        if let Ok(mut guard) = SERVER_PID.lock() {
            *guard = None;
        }
        *SERVER_STDIN.lock().await = None;
        let _ = wait_events
            .send(CoreEvent::ServerState(ServerState::Stopped { exit_code }))
            .await;
    });

    println!("服务端进程 PID: {pid}");
    Ok(())
}

/// 优雅停止：向 stdin 写入 `stop`；没有 stdin 时按 PID 终止。
pub async fn stop() -> Result<(), String> {
    let mut guard = SERVER_STDIN.lock().await;
    if let Some(stdin) = guard.as_mut() {
        stdin
            .write_all(b"stop\n")
            .await
            .map_err(|e| format!("发送 stop 失败: {e}"))?;
        stdin.flush().await.map_err(|e| e.to_string())?;
        return Ok(());
    }
    drop(guard);

    let pid = {
        let guard = SERVER_PID.lock().map_err(|e| e.to_string())?;
        guard.ok_or("没有运行中的服务端")?
    };
    #[cfg(windows)]
    {
        std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn new_server_binds_loopback_by_default() {
        let dir = temp_dir("loopback");
        write_properties(&dir, 25565, true).await.unwrap();
        let text = std::fs::read_to_string(dir.join("server.properties")).unwrap();
        assert!(text.lines().any(|line| line == "server-ip=127.0.0.1"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn worldgen_temp_names_do_not_collide() {
        let base = Path::new("cache/server");
        assert_ne!(worldgen_temp_path(base), worldgen_temp_path(base));
    }

    #[tokio::test]
    async fn localhost_server_properties_cannot_keep_wildcard_binding() {
        let dir = temp_dir("existing-binding");
        std::fs::write(dir.join("server.properties"), "server-ip=\n").unwrap();
        write_properties(&dir, 25565, false).await.unwrap();
        let text = std::fs::read_to_string(dir.join("server.properties")).unwrap();
        assert!(text.lines().any(|line| line == "server-ip=127.0.0.1"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "deepslate-server-test-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn finds_server_download_through_inheritance() {
        let root = temp_dir("inherit");
        let versions = root.join("versions");
        std::fs::create_dir_all(versions.join("26.3")).unwrap();
        std::fs::create_dir_all(versions.join("fabric-loader-0.20.1-26.3")).unwrap();
        std::fs::write(
            versions.join("26.3/26.3.json"),
            r#"{"id":"26.3","downloads":{"server":{"url":"https://example.com/server.jar","sha1":"abc"}}}"#,
        )
        .unwrap();
        std::fs::write(
            versions.join("fabric-loader-0.20.1-26.3/fabric-loader-0.20.1-26.3.json"),
            r#"{"id":"fabric-loader-0.20.1-26.3","inheritsFrom":"26.3","mainClass":"KnotClient"}"#,
        )
        .unwrap();

        let (base, url, sha1) =
            find_server_download(&versions, "fabric-loader-0.20.1-26.3").unwrap();
        assert_eq!(base, "26.3");
        assert_eq!(url, "https://example.com/server.jar");
        assert_eq!(sha1, "abc");

        let err = find_server_download(&versions, "missing").unwrap_err();
        assert!(err.contains("请先下载"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn properties_are_created_and_updated_in_place() {
        let dir = temp_dir("props");
        write_properties(&dir, 25565, false).await.unwrap();
        let text = fs::read_to_string(dir.join("server.properties"))
            .await
            .unwrap();
        assert!(text.contains("server-port=25565"));
        assert!(text.contains("online-mode=false"));
        assert!(text.contains("level-name=world"));

        fs::write(
            dir.join("server.properties"),
            "motd=My Server\nserver-port=25565\ncustom=keep\n",
        )
        .await
        .unwrap();
        write_properties(&dir, 25566, true).await.unwrap();
        let text = fs::read_to_string(dir.join("server.properties"))
            .await
            .unwrap();
        assert!(text.contains("motd=My Server"));
        assert!(text.contains("custom=keep"));
        assert!(text.contains("server-port=25566"));
        assert!(text.contains("online-mode=true"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn eula_requires_acceptance_and_is_written() {
        let dir = temp_dir("eula");
        write_eula(&dir).await.unwrap();
        assert_eq!(
            fs::read_to_string(dir.join("eula.txt")).await.unwrap(),
            "eula=true\n"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn world_type_and_gamemode_mappings() {
        assert_eq!(WorldType::Normal.level_type(), "minecraft:normal");
        assert_eq!(WorldType::Flat.level_type(), "minecraft:flat");
        assert!(WorldType::Flat.generator_settings().contains("\"layers\""));
        assert_eq!(game_mode_id("survival"), 0);
        assert_eq!(game_mode_id("creative"), 1);
        assert_eq!(game_mode_id("adventure"), 2);
        assert_eq!(game_mode_id("spectator"), 3);
        assert_eq!(game_mode_id("unknown"), 0);
    }

    #[test]
    fn detects_fabric_profile_libraries() {
        let root = temp_dir("fabric-detect");
        let versions = root.join("versions");
        let libraries = root.join("libraries");
        std::fs::create_dir_all(versions.join("fabric-loader-0.20.1-26.3")).unwrap();
        std::fs::create_dir_all(versions.join("26.3")).unwrap();
        std::fs::write(
            versions.join("fabric-loader-0.20.1-26.3/fabric-loader-0.20.1-26.3.json"),
            r#"{"id":"fabric-loader-0.20.1-26.3","mainClass":"net.fabricmc.loader.impl.launch.knot.KnotClient","libraries":[{"name":"org.ow2.asm:asm:9.10.1"},{"name":"net.fabricmc:fabric-loader:0.20.1"}]}"#,
        )
        .unwrap();
        std::fs::write(
            versions.join("26.3/26.3.json"),
            r#"{"id":"26.3","mainClass":"net.minecraft.client.main.Main","libraries":[]}"#,
        )
        .unwrap();

        let libs = detect_fabric_libraries(&versions, &libraries, "fabric-loader-0.20.1-26.3");
        assert_eq!(libs.len(), 2);
        assert!(libs[0].ends_with("org/ow2/asm/asm/9.10.1/asm-9.10.1.jar"));
        assert!(libs[1].ends_with("net/fabricmc/fabric-loader/0.20.1/fabric-loader-0.20.1.jar"));
        assert!(detect_fabric_libraries(&versions, &libraries, "26.3").is_empty());
        assert!(detect_fabric_libraries(&versions, &libraries, "missing").is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
