# 深板岩启动器（Deepslate Launcher）

纯 Rust 编写的 Minecraft Java 版启动器：`core` 无 GUI 依赖，`gui-iced` 使用 iced 0.14。
视觉识别：牛皮纸黄 × 石板灰，亮/暗双主题。

## 功能

- Microsoft 登录（设备码/回调 URL 粘贴）与离线档案
- 版本列表、并发下载（SHA1 校验、原子落盘、重试）
- **首次启动 EULA 授权**：从官方页面抓取原文并缓存，必须滚动到底 + 勾选，
  写入 `<root>/eula.txt` 后才能启动客户端/服务端/建世界
- 全局**地图库**：地图独立于游戏版本，`game/saves` 链接到 `<root>/maps`
- **Fabric 加载器安装与启动**（`meta.fabricmc.net`，支持选择 MC 版本与加载器版本）
- 已安装版本扫描（原版 / Fabric / Quilt / Forge / NeoForge 识别）
- **默认中文 + 跳过辅助功能引导**（首次接管实例时写入 `options.txt`，之后尊重游戏内修改）
- **地图库管理**：名称 / 模式 / 难度 / 版本 / 最后游玩 / 大小，原生文件选择导入 zip/文件夹、导出 zip、单图备份/恢复、重命名、复制、删除到回收站
- **启动器内新建世界**：后台原版服务端生成（正常/超平坦、种子、生存/创造、作弊），直接纳入地图库
- **快速进入存档 / 快速加入服务器**（26.3 的 `--quickPlaySingleplayer` / `--quickPlayMultiplayer`）
- **增强体验（Minecraft 26.3+）**：Bridge 汇报资源/世界加载阶段与耗时，启动器可直接进入地图或服务器
- **增强客户端设置**：启动器选择语言、GC 方案与最大内存；新建单人世界时可设难度和常用游戏规则
- **增强服务器收藏**：保存命名地址，查询 MOTD、版本、在线人数和延迟后快速连接
- **本地服务端**：下载 `server.jar`、写 EULA/`server.properties`、启动/停止、日志查看、就绪后自动加入
- **高级启动开关**：在启动页「高级启动」折叠区配置禁用多人/聊天、全屏、窗口尺寸与图形后端（26.3 参数）
- **模组管理**：解析 `fabric.mod.json`，启用/禁用（`.jar.disabled`）、删除到回收站
- **标题界面启动器名称**（vanilla 下把 `version.json` 覆盖到 classpath 最前；Fabric 受 Knot 限制暂不生效）

## 构建与运行

```bash
cargo run -p gui-iced     # 启动 GUI（二进制 deepslate-iced）
cargo build               # 构建整个 workspace
cargo test                # 单元测试
```

### Bridge 打包

增强体验的 Bridge 源码位于 `bridge/`，发布 JAR 位于 `core/assets/` 并随 Rust 启动器编译嵌入。用户安装启动器后不需要单独下载 Bridge；首次启用增强体验时，启动器会把内嵌 JAR 校验后写入独立组件代次。Minecraft 和 Fabric API 仍按需下载。

修改 Bridge 源码或资源后，先从仓库根目录重建 JAR，再构建启动器：

```bash
./bridge/package.sh
cargo test -p deepslate-core --lib
cargo build --release --workspace
```

提交时把 Bridge 源码和更新后的 `core/assets/deepslate-bridge-*.jar` 一起提交。Gradle 缓存与构建输出不进入 Git。

数据默认位于 `~/Games/ObsidianLauncher`（沿用历史路径，避免已有游戏文件失效）。

## 开发文档

- [增强体验交接](docs/current/HANDOFF-enhanced-experience.md)：Bridge、好友同步与增强模式的现状和限制
- [Bridge 分发设计](docs/current/DESIGN-bridge-distribution.md)：源码/JAR 入库、内嵌发布与首次增强安装流程
- [UI 体验优化 spec](docs/current/SPEC-ui-optimization.md)：7 项导航、启动页折叠区、世界卡菜单与离屏截图验收

## Fabric 安装

GUI「启动」页 →「Fabric 加载器」卡片：选择基准 MC 版本与 Fabric 版本，点击「安装 Fabric」。
安装器会：

1. 确保原版基础版本完整（版本 JSON / 客户端 jar / 库 / 资源）；
2. 拉取 `https://meta.fabricmc.net/v2/versions/loader/<game>/<loader>/profile/json`；
3. 下载 profile 声明的 maven 库（ASM、sponge-mixin、intermediary、fabric-loader）；
4. 写入 `versions/fabric-loader-<loader>-<game>/` 并创建实例 `mods/` 目录。

启动器只负责把 Fabric 的库放进 classpath、切换主类为
`net.fabricmc.loader.impl.launch.knot.KnotClient` 并保留 `-DFabricMcEmu=...`；
**mod 的字节码注入由 Fabric Loader 在游戏进程内完成**，启动器不修改客户端 jar。
mod 放入实例目录的 `mods/` 即可（当前单实例布局是 `root/game/mods/`）。

## EULA 与授权

客户端首次下载完成后、下一次启动前会在主窗口弹出 EULA 模态（不是日志窗口）：

1. 从 `https://www.minecraft.net/en-us/eula` 抓取页面，只取 `#main-content` 正文；
2. 原始 HTML、提取文本与 SHA-1 一起缓存到 `<root>/legal/`，断网时可回退缓存并提示；
3. 必须把正文滚动到底、再勾选同意，按钮才可用；
4. 同意后写入 `<root>/eula.txt`（内容与服务端 `eula.txt` 相同），并继续原本的启动动作。
5. 接受状态同时记录正文 SHA-1（`<root>/legal/eula-accepted.json`）。启动时检查
   官方正文哈希；协议实质更新会让旧同意失效，必须重新阅读到底并勾选。

本地开服和“新建世界”共用同一个授权标记；CLI 使用 `--accept-eula` 显式同意。

### 无 GUI 调试

```bash
deepslate-iced --install-fabric 1.21.1 --loader 0.16.14 --root /tmp/mc
deepslate-iced --print-command fabric-loader-0.16.14-1.21.1 --root /tmp/mc --java /path/to/jdk
deepslate-iced --launch fabric-loader-0.16.14-1.21.1 --accept-eula --root /tmp/mc --java /path/to/jdk
```

`--print-command` 会补齐缺失的库并打印完整启动计划（主类 / classpath / JVM 参数 / 游戏参数），
不启动进程；`--launch` 直接启动并把握手日志写 stderr，方便排障。

`--launch` 与服务端/建世界命令一样需要每次显式 `--accept-eula`（CLI 不走 GUI
的协议更新检查，不能拿旧标记静默放行）；该参数会写入 `<root>/eula.txt`，
下次 GUI 启动会自动读取。

```bash
deepslate-iced --list-saves 26.3 --root /tmp/mc
deepslate-iced --launch 26.3 --accept-eula --root /tmp/mc --quick-save world
deepslate-iced --launch 26.3 --accept-eula --root /tmp/mc --quick-server 127.0.0.1:25565
deepslate-iced --rename-save world --new-name "新世界" --saves-version 26.3 --root /tmp/mc
deepslate-iced --copy-save world --new-name "副本" --saves-version 26.3 --root /tmp/mc
deepslate-iced --trash-save world-copy --saves-version 26.3 --root /tmp/mc
deepslate-iced --host-server 26.3 --accept-eula --port 25566 --memory 2048 --root /tmp/mc
deepslate-iced --create-world 26.3 --accept-eula --name "新世界" --seed 12345 --flat --root /tmp/mc
deepslate-iced --generate-map 26.3 --accept-eula --name "新世界" --seed 12345 --flat --root /tmp/mc
deepslate-iced --list-maps --root /tmp/mc
deepslate-iced --import-map /tmp/world.zip --root /tmp/mc
deepslate-iced --export-map <map-id> --root /tmp/mc
deepslate-iced --backup-map <map-id> --root /tmp/mc
deepslate-iced --list-map-backups <map-id> --root /tmp/mc
deepslate-iced --restore-map <map-id> --root /tmp/mc
deepslate-iced --launch 26.3 --accept-eula --root /tmp/mc --quick-map <map-id>
deepslate-iced --list-mods fabric-loader-0.20.1-26.3 --root /tmp/mc
```

## 地图库与快速进入

「世界」页管理全局地图库。地图库位于 `<root>/maps/<map-id>/`，内部就是标准
Minecraft 世界目录，额外带一个游戏会忽略的 `map.json`；游戏实例的
`<game>/saves` 是指向 `maps/` 的链接。旧版 `saves/` 会在首次打开世界页时
导入地图库，并保留为 `saves.legacy-<时间戳>`。

- **进入**：以 `--quickPlaySingleplayer <map-id>` 启动，跳过主菜单直接加载世界
- **导入**：支持 zip（根目录 / `world/` / 单层目录包含 `level.dat`）和世界文件夹
- **导出**：把地图目录打成 zip，包含 `map.json`，可再导入
- **备份/恢复**：快照写入 `<root>/backups/maps/<map-id>/`，保留最近 5 份；恢复会用最近备份原子替换当前地图
- **重命名**：只改 `Data.LevelName`，文件夹 id 不变
- **复制**：生成新 map id / 新文件夹，重置 `LastPlayed` 与 `session.lock`
- **删除**：移动到 `maps/.deepslate-trash/`，可在世界页清空回收站
- **新建**：填写名称/种子/模式/超平坦/作弊，后台服务端生成后**直接纳入地图库**；
  首次需要下载 `server.jar`（约 60MB）并同意 EULA
- **崩溃恢复**：生成任务写 `.job.json`（PID）和 `.complete.json`（完成标记）；
  启动器启动时清理孤儿 server 与半成品，已完成但未纳入的世界会自动恢复

旧版的版本隔离/存档隔离不再使用；`game/saves` 被统一接管为地图库链接。
`map.json` 记录来源、生成参数、`DataVersion` 和最后游玩时间，便于后续升级/备份。
版本选择器展示正式版清单和已安装版本；快照不在当前产品路径中。

「启动」页的“快速加入服务器”使用 `--quickPlayMultiplayer <host:port>`，
历史地址保存在 `config.json` 的 `recent_servers`。

## 本地服务端

「服务器」页可以安装/校验 `server.jar`、配置端口/内存/正版验证、启动与停止，
日志实时滚动；“启动并加入”会在服务端输出 `Done (…)` 后自动以
`--quickPlayMultiplayer 127.0.0.1:<port>` 启动客户端。

- 服务端目录：`<root>/servers/<version>/`，`server.properties` 中除
  `server-port` / `online-mode` 外的键会保留
- EULA 必须先在主窗口弹窗中阅读到底并勾选；授权凭据是 `<root>/eula.txt`，
  服务端目录的 `eula.txt` 由安装/启动流程按同款格式写出
- 选中的版本是 Fabric 时，会沿版本 JSON 的库列表启动 **Fabric Server
  (KnotServer)**，并加载 `servers/<version>/mods/`；原版版本则直接 `-jar server.jar`

## 模组

「模组」页读取当前版本实例的 `mods/`：

- 解析 `fabric.mod.json` 的 `id/name/version/description/authors/environment`
- 启用/禁用通过重命名 `.jar` ↔ `.jar.disabled` 实现（Fabric 只加载 `.jar`）
- 删除移动到 `mods/.deepslate-trash/`，可恢复；非 Fabric jar 也会列出

## 目录结构

```
core/src/
  auth.rs        Microsoft OAuth 与 Xbox/XSTS 链
  backup.rs      旧版存档备份（兼容/调试；产品使用地图导出）
  branding.rs    标题界面 version.json 覆盖（vanilla）
  config.rs      配置与目录解析
  download.rs    原版下载 + 共享并发下载器
  eula.rs        EULA 抓取、正文解析、缓存与 `<root>/eula.txt`
  fabric.rs      Fabric 元数据与安装
  game_options.rs options.txt 默认值（中文 / 跳过引导）
  launch.rs      版本链合并、规则/占位符求值、启动计划与进程
  maps.rs        地图库：map.json / 导入导出 / 列表 / 回收站
  mods.rs        mods/ 扫描、fabric.mod.json、启用/禁用/回收站
  nbt.rs         NBT 读写（level.dat）
  net.rs         统一 HTTP（UA/超时/重试）
  profiles.rs    用户档案
  runtime.rs     game/instance/maps/cache 路径与 saves 链接/迁移
  saves.rs       存档解析 / 重命名 / 复制 / 回收站
  server.rs      本地服务端安装 / 启动 / 停止 / 日志
  versions.rs    本地版本扫描与加载器识别
  worldgen.rs    外置 server.jar 世界生成并纳入地图库
gui-iced/src/
  main.rs        应用状态、消息、页面与 CLI 调试入口
  style.rs       亮/暗主题
  log_window.rs  独立日志窗口子进程
```

版本 JSON 的合并语义（子版本优先、库去重、参数父先子后）参考 HMCL；
Fabric 安装路径与 `inheritsFrom` 版本布局参考 Fabric 官方 installer 与
portablemc / SwiftLauncher 的做法。

## License

MIT
