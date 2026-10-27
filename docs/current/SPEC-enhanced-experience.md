# 增强体验：可行性分析与推进方案

日期：2026-10-27。状态：方案建议，尚未实现设置项、桥接协议或 Fabric Mod。

## 1. 结论与产品行为

可以实现。现有 Rust / iced 启动器保留为用户入口，新增一个自研的客户端 Fabric Mod（暂名 `deepslate-bridge`），负责把启动器操作接入 Minecraft 的世界生命周期。

“启动器即客户端”可以做到统一操作流程：选世界、新建、管理存档均在启动器完成；游戏直接进入目标世界；保存退出后关闭游戏进程，恢复启动器页面。技术上仍是 Rust 启动器和 Java 游戏两个进程。把 Minecraft 的渲染窗口嵌进 iced 属于另一项跨平台工程，本方案不依赖窗口嵌入。

建议「设置 → 游戏与启动」新增「增强体验」，默认关闭以兼容现有配置。开启后的行为：

- 游戏版本固定为 **Minecraft Java 26.3 正式版**，不跟随 `manifest.latest.release` 切换其他 MC 版本。
- 自动安装/维护适配 26.3 的最新稳定 Fabric Loader、正式发布的 Fabric API，以及配套的 Bridge Mod。
- 进入、新建世界统一走增强模式；没有目标世界时先展示启动器地图库，不把原版标题界面当成入口。
- 从世界中选择保存退出后，完成保存并关闭 Java 客户端；启动器显示地图库及更新后的最后游玩信息。
- 增强模式实例与普通模式的 `mods/options` 分开，复用下载资源与既有地图库。
- 开关变更在下次游戏会话生效。关闭增强体验不删除世界或用户模组。

这里把“退出客户端而非回到主页面”理解为：**退出世界后结束 Minecraft 进程，回到启动器**。死亡后的重生、打开暂停菜单和资源重载不属于退出世界。

## 2. 本次实际检查的证据

### 2.1 当前项目

| 能力 | 当前实现 | 本次建议 |
| --- | --- | --- |
| 配置与设置页面 | `core/src/config.rs`、`gui-iced/src/views.rs` | 增加持久化开关和增强模式状态 |
| Fabric 安装 | `core/src/fabric.rs` | 复用 Loader/profile/library 下载；补 Fabric API 自动安装 |
| 启动、Java 检查、进程等待 | `core/src/launch.rs` | 增加桥接会话参数与正常退出命令 |
| 启动动作入口 | `gui-iced/src/main.rs` 的 `start_launch` | 启动、继续游戏、地图进入、联机统一经过模式解析 |
| 运行目录与 saves 链接 | `core/src/runtime.rs` | 引入增强模式独立实例，复用 `maps` |
| 地图库、回收站、备份、锁 | `core/src/maps.rs` | 保留并扩展会话协调，避免重复写一套存档管理 |
| 现有世界生成 | `core/src/worldgen.rs` → 原版服务端 | 增强模式改由客户端创建；普通模式保留原路径 |
| 游戏事件 | `core/src/events.rs` | 保留进程状态，增加握手、加载世界、保存、失败等会话事件 |

当前 `GameState::Running` 在子进程启动后发送，不能代表游戏资源已加载或世界已进入。需要增加独立的会话状态。

当前游戏启动期间已持有 `maps::lock_library`，并在进入已有地图前备份。这些能力可以复用。改成 IPC 打开世界时，也必须把目标地图交给启动准备流程，否则仅依赖当前 `quick_play` 字段的备份分支会被绕过。

### 2.2 用户提供的网易 JAR

文件：`4689596882874350247@3@0.jar`。使用 ZIP 元数据、`javap` 和 CFR 做静态检查，没有加载或运行其 Mod。

`META-INF/neoforge.mods.toml` 指定：

- 模组：`netease_official`。
- Minecraft：`[1.21.10]`。
- NeoForge：`[21.10.64,)`。

因此它不是可直接放入 26.3 Fabric 的 Mod。可借鉴的调用关系已经核实：

| 类 | 检查到的行为 |
| --- | --- |
| `ClientNetworkHandler` | 在客户端 tick 从接收队列取消息，按消息编号派发处理器 |
| `GuiOpenEventHandler` | 首次初始化标题界面时向启动器请求操作；后续满足条件再次回到标题界面时关闭客户端 |
| `ReplyStartExistSingle` | 调用 `WorldOpenFlows.openWorld` 打开指定世界 |
| `ReplyNewSingleV2` | 组装游戏模式、种子、世界预设等参数，调用 `createFreshLevel` |
| `ReplyCloseMinecraft` | 响应关闭命令，调用客户端 `close()` |
| `MixinLoadingOverlay` | 将加载画面绘制转交给自定义包装器 |

该 JAR 引用的 `network/socket`、`network/common` 等基础类没有包含在包内。可据此确认桥接思路，但无法仅凭这个包还原完整通信实现与所有启动器行为。

自研实现只参考职责划分与游戏 API 调用关系，独立编写协议、Mod 和画面。退出逻辑按 26.3 的正常生命周期实现。

### 2.3 用户提供的 Minecraft 26.3 JAR

目录 `/home/qaqtamsy/Projects/dwq` 中实际包含 `26.3.jar`，没有 `.java` 源文件。本次按类提取并反编译到临时目录，原始 JAR 保持原样。

JAR 的 `version.json` 表明：`id=26.3`、`stable=true`、`java_version=25`、`world_version=5023`。主要接入点：

| 26.3 类 / 方法 | 已核实用途 |
| --- | --- |
| `Minecraft.isGameLoadFinished()` | 判断初次资源加载已完成；还需结合屏幕状态判断可执行操作 |
| `Minecraft.createWorldOpenFlows()` | 取得世界创建和打开流程 |
| `WorldOpenFlows.openWorld(String, Runnable)` | 通过正式流程加载世界，并处理取消、损坏、版本兼容等分支 |
| `WorldOpenFlows.createFreshLevel(String, LevelSettings, WorldOptions, Function, Screen)` | 从参数创建世界并启动内置服务器 |
| `PauseScreen` → `Minecraft.disconnectFromWorld(Component)` | 暂停菜单的正常退出世界入口 |
| `Minecraft.disconnect(Screen, boolean, boolean)` | 调用内置服务器 `halt(false)`，循环等待 `server.isShutdown()`，然后清理世界状态 |
| `Minecraft.stop()` | 将主循环运行标记设为 false |
| `Main.main` → `Minecraft.run()` → `exitWorldAndClose()` | 主循环结束后退出世界并释放客户端资源 |
| `LoadingOverlay.extractRenderState(...)` | 26.3 加载画面的绘制状态提取入口 |
| `LoadingOverlay.tick()` | 检查资源加载结果、执行完成回调、推进结束状态 |

26.3 中屏幕管理由 `Minecraft.gui` / `Gui.setScreen` 等接口承担。不能直接照搬旧版 `Minecraft.setScreen`、`mc.screen` 或 `LoadingOverlay.render` 的注入示例。

原版 Quick Play 的单人进入最终也调用 `WorldOpenFlows.openWorld`。它可用于早期验证，但增强模式正式链路应由 Bridge 统一打开世界，避免 Quick Play 和 IPC 同时触发两次加载。

## 3. 推荐架构

```mermaid
flowchart LR
    UI[iced 启动器：地图库与启动页面] --> Core[Rust：安装 / 会话 / 地图库]
    Core <-->|本机 TCP / JSON / 会话鉴权| Bridge[Fabric Bridge Mod]
    Bridge --> MC[26.3 世界加载 / 创建 / 保存退出]
    Core --> Maps[共享 maps 与备份]
    MC --> Saves[增强实例 saves 链接]
    Saves --> Maps
```

Rust 负责安装、地图列表、导入导出、备份、回收站及子进程监控。Mod 负责游戏线程内的世界创建/加载、状态回报和正常关闭。服务器不需要安装 Bridge，单人游戏的内置服务器随客户端正常运行。

建议目录示意：

```text
<root>/game/versions|libraries|assets/   共享下载文件
<root>/instances/enhanced-26.3/         增强模式 --gameDir
    mods/                             Fabric API、Bridge、用户启用的模组
    options.txt
    saves -> <root>/maps
<root>/maps/<map-id>/                   保留当前地图库
<root>/config/enhanced-runtime.lock.json
<root>/cache/bridge-sessions/<id>/
```

新增实例需同时检查现有模组页、日志目录、options 写入位置、CLI 和服务端目录解析，不能只更改 `--gameDir`。共享游戏文件仍通过绝对路径传给启动计划。现有用户模组的迁入应有明确入口，不自动把普通实例中的所有 JAR 混入增强实例。

## 4. 自动安装与更新

本次联网读取官方元数据确认：Mojang manifest 包含正式版 26.3；26.3 Loader 列表首个稳定项为 `0.20.1`；Fabric API Maven 元数据中可见 `0.165.0+26.3`。这些值只记录本次查询结果，不应写成永久的“最新版”。

增强模式的解析顺序建议为：

1. 固定请求 **26.3**，检查完整客户端、依赖库、资源与 Java 25 运行环境。
2. 从 Fabric 的 26.3 Loader 元数据选择最新稳定兼容项；不盲目使用全局 Loader 列表兜底。
3. 查找适用于 26.3、Fabric 的 Fabric API 正式发布包，并核验包内依赖约束。不能按全局最新版本号下载其他 MC 版本的 API。
4. 配置与 Bridge 声明兼容的组合，校验下载摘要，并检查同 ID 的重复模组。
5. 先写临时文件，全部成功后原子切换运行配置；记录 MC、Loader、API、Bridge、协议版本与摘要。
6. 检查新版本时只更新变化的组件；更新失败保留已验证可用的组合，并在界面说明。没有可用组合时明确报告失败。

“只下载 26.3”指只安装这一种 MC 版本；运行它仍需要相应资源、库、Fabric 组件和 Java。Fabric API 是单独的 Mod JAR，现有 Loader 安装器没有自动安装它。

Bridge 的构建依赖要固定，运行时更新的 Loader/API 也要检查兼容范围。采用官方适用于非混淆版本的 `net.fabricmc.fabric-loom`，按 Java 25 构建；具体 Gradle/Loom 版本在建立 Mod 工程时选择并锁定。

## 5. 桥接协议与会话状态

初版采用跨平台本机 TCP，启动器监听 `127.0.0.1` 随机端口，Mod 主动连接。JSON 使用明确的消息边界和大小上限。会话凭据通过仅当前用户可读的临时启动描述文件传入，避免把凭据写进命令日志。

每条消息携带协议版本、会话 ID 和请求 ID；完成握手后才能接受命令。游戏操作通过 `Minecraft.execute` 或游戏线程队列执行，网络线程只解析消息。

建议最小命令：

- `OpenWorld(map_id)`。
- `CreateWorld(map_id, name, seed, mode, difficulty, preset, cheats, ...)`。
- `GetSessionState`。
- `QuitClient`：调用正常停止路径，回报正在保存，并由启动器等待进程结束。

建议最小事件：`Hello`、`Ready`、`WorldLoading`、`WorldEntered`、`WorldLeaving`、`WorldClosed`、`OperationFailed`。进程退出由 Rust 的 `child.wait()` 独立确认。

```text
准备依赖 → 启动 JVM → Bridge 握手 → 游戏可操作
    → 加载 / 创建世界 → 世界已进入 → 保存并退出
    → 内置服务器关闭 → 客户端退出 → 启动器刷新地图库
```

事件需关联对应会话，重复请求不能再次创建世界或再次进入。握手有超时；创建/加载世界应持续上报状态，不能因为一个固定短超时就强行结束进程。

启动器意外关闭或桥接暂时断开时，游戏保留正常保存退出能力；不能把连接丢失直接处理成强杀游戏。

## 6. 存档创建、删除与正常退出

**创建：** 增强模式将启动器表单转换成 `LevelSettings`、`WorldOptions` 和世界预设，通过 `createFreshLevel` 执行。默认动作建议为“创建并进入”；若后续需要“仅创建”，在初始化完成后正常保存退出。使用当前 Fabric 环境的注册表和数据包，不用原版服务端预生成带模组内容的世界。

创建任务先记录待完成状态。只有世界初始化和有效存档写入得到确认后才标记完成；失败或崩溃留下的目录进入待恢复状态，不能显示为创建成功，也不能直接删除可能已生成的数据。

**删除：** 没有游戏进程时直接复用 `maps::trash`，无需为删除启动 JVM。游戏运行时通过 Mod 确认世界状态；当前世界必须先正常保存退出并释放占用，再由启动器移动到回收站。初版可以继续采用现有“游戏期间锁定整个地图库的修改操作”的规则，后续再细化到单图锁。

Mod 下发的世界标识只接受地图库 map ID，不接受任意绝对路径。目录校验需要考虑现有 saves 链接，并防止世界条目自身逃逸地图库。

**退出：** 初版优先拦截受管会话的 `disconnectFromWorld` 正常退出动作：发出保存状态，走原版世界关闭流程，完成后请求 `Minecraft.stop()`。由主循环及 `exitWorldAndClose()` 完成客户端清理。具体注入点在 Mod 工程中通过编译及实机验证确定。

不能只看到 `TitleScreen` 就关闭游戏：首次启动、资源包恢复、加载取消等也可能到达标题界面。应结合受管会话状态和退出原因判断。损坏存档、数据包错误和版本兼容确认需保留可恢复的错误流程，并同步错误给启动器。

## 7. 自定义启动页面

分开覆盖两个阶段：

- **启动器页面：** 下载、校验、启动 Java、等待 Mod、读取/生成世界等阶段与真实进度；增强模式隐藏手动版本和 Loader 选择，保留组件详情用于排障。
- **游戏画面：** 自定义等待画面、资源加载画面、世界加载与保存提示，使用项目现有牛皮纸黄与石板灰视觉。

游戏窗口建立前的 JVM 和 Loader 发现阶段由启动器显示状态；Mod 接管之后才能绘制游戏内界面。资源加载可以使用实际资源加载进度，其他阶段显示阶段状态，不拼出虚假的总体百分比。

26.3 的 `LoadingOverlay.tick()` 承担完成回调和异常检查。定制绘制时必须保留这些逻辑；只替换视觉或使用等价 overlay，不能简单取消整个加载流程。先跑通进入与退出，再做加载画面的 Mixin，减少同时排查的变量。

## 8. 实施顺序与每阶段验收

| 阶段 | 交付 | 验收条件 |
| --- | --- | --- |
| 1：设置与运行组合 | 开关、固定 26.3、Loader/API 自动安装、独立实例、组件锁定记录 | 旧配置可加载；首次安装成功；重复启动复用资源；失败不破坏原可用组合；其他 MC 版本不被自动下载 |
| 2：Bridge 最小闭环 | 独立 Fabric 工程、本机握手、已有世界进入、正常退出、启动器状态 | 启动器选图 → 直接进入 → 保存退出 → Java 结束 → 启动器地图更新；重新进入验证修改已保存 |
| 3：创建与存档协调 | 客户端创建世界、状态回报、失败恢复、删除互斥 | 验证种子/模式/预设；当前世界不可删除；退出后可进回收站；创建中崩溃不会误标成功 |
| 4：完整界面 | 启动器页面、游戏加载/等待/保存画面、错误反馈 | 初次启动不落入可操作的原版主菜单；资源重载/错误流程正常；小窗口可用 |
| 5：跨平台与交付 | Bridge 构建打包、协议兼容检查、升级/回退 | Linux/Windows/macOS 分别验证；用户模组冲突可定位；关闭增强模式后普通启动可用 |

第一条实机闭环是关键里程碑：**固定 26.3 + Fabric/API/Bridge → 从启动器进入已有世界 → 正常保存并结束客户端**。它通过后再扩展创建世界与画面。

## 9. 验证范围与外部依据

本次完成源码阅读、两个 JAR 的元数据检查、指定类的字节码及反编译分析、官方版本元数据查询。没有构建 Bridge、运行提供的网易 Mod、启动 Minecraft 或验证桌面交互；尚不能把以上方案称为已完成的功能。

外部依据：

- [Minecraft Java 26.3 官方发布说明](https://www.minecraft.net/en-us/article/minecraft-java-edition-26-3)。
- [Mojang 版本 manifest](https://piston-meta.mojang.com/mc/game/version_manifest_v2.json)。
- [Fabric 26.3 说明](https://fabricmc.net/2026/09/15/263.html)。
- [Fabric 26.3 Loader 元数据](https://meta.fabricmc.net/v2/versions/loader/26.3)。
- [Fabric API Maven 元数据](https://maven.fabricmc.net/net/fabricmc/fabric-api/fabric-api/maven-metadata.xml)。
- [Fabric Loom 文档：26.1+ 的非混淆版本插件](https://docs.fabricmc.net/develop/loom/)。
- [Fabric 事件文档](https://docs.fabricmc.net/develop/events/)：公共事件优先，缺失接入点再使用 Mixin。当前文档页面标为 26.2，26.3 的具体签名以提供的 26.3 JAR 及后续构建验证为准。
