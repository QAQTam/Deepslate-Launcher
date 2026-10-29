# 增强体验交接 — 2026-10-27

状态：**未完成；按用户指示暂缓 Windows 适配。** 本文记录工作树当前状态，规格仍以 [SPEC-enhanced-experience.md](./SPEC-enhanced-experience.md) 为准。不要把下面的局部实机结果当作整个规格通过。

## 工作树与交付物

工作树包含大量本任务之前及本任务中的未提交、未跟踪文件；不要整体重置、覆盖或清理。原 `docs/current/HANDOFF.md` 是较早的 EULA/UI 交接，并非本功能的现状。用户提供的 `4689596882874350247@3@0.jar` 和 `/home/qaqtamsy/Projects/dwq/26.3.jar` 保持原样；没有运行网易 JAR。

- `bridge/`：独立 Fabric 26.3 客户端模组，Java 25、Loom 1.19.3；`bridge/package.sh` 构建并更新内嵌 JAR。分发与 Git 追踪策略见 [Bridge 分发设计](./DESIGN-bridge-distribution.md)。
- `core/assets/deepslate-bridge-0.5.0.jar`：当前 Rust 内嵌构建产物，必须随 Bridge 源码变化重新构建；与 `bridge/build/libs/deepslate-bridge-0.5.0.jar` SHA-256 均为 `8c85c23e4763cc0bf5faf0d0df92a2847ddb3a9a983bf8ce7b4141695e7e3e35`。0.4.0 及更早 JAR 仅保留历史，不再嵌入。该摘要只是当前状态，不是长期版本号或信任锚。
- `core/src/enhanced.rs`、`fabric.rs`：26.3 的组件解析、校验、代次和安装；`config/enhanced-runtime.lock.json` 指向组件代次。
- `core/src/bridge.rs`、`launch.rs`、`events.rs`：本机 TCP/JSON 握手、受管会话、世界操作与进程生命周期。
- `core/src/runtime.rs`、`maps.rs`、`mods.rs`：增强实例、共享地图库、备份、待恢复创建任务与模组目录。
- `gui-iced/src/main.rs`、`views.rs`：设置、世界、联机、状态及手动恢复入口。
- `core/tests/enhanced_live.rs`：需显示环境、Java 25 和临时测试根的 ignored 实机测试。

## 已实现及已有证据

- 设置默认关闭并兼容旧配置；增强实例为 `<root>/instances/enhanced-26.3`，共享游戏资源和 `<root>/maps`，普通实例的模组未自动迁入。
- 增强客户端固定 26.3，从该游戏版本的 Fabric 元数据选择稳定 Loader；解析 26.3 正式 Fabric API，核验官方 SHA-1、JAR 标识及部分依赖约束；Bridge 内嵌并校验；模组同 ID 冲突会报错。组件代次写完后切换锁文件，安装失败尝试使用已验证组合。Windows 下这些安装与回退行为尚未验证。
- Linux 上曾在隔离目录完成一次真实首次安装；联网解析真实 Fabric 元数据通过。已装资源被后续测试复用。测试当时选择 Loader `0.20.1`、API `0.165.0+26.3`，不代表永久最新版。
- Linux/Wayland 实机：使用**复制**的既有世界，经 Bridge 进入、由启动器请求保存退出、Java 结束，并再次进入同一副本；没有验证原世界或用户账号数据。最近一次已有世界实机测试也在资源加载画面接入后通过。
- Linux/Wayland 实机：客户端创建并进入测试世界，保存退出后读取存档，确认名称、创造模式、作弊、种子 `42` 和超平坦预设；26.3 的世界生成设置位于 `data/minecraft/world_gen_settings.dat`，不在 `level.dat` 中。
- 创建前写待完成记录；未完成任务不作为普通世界显示。地图库有手动恢复入口，检查 `level.dat` 与世界生成设置，恢复需持地图库锁；损坏或不完整数据不自动删除。已有单元测试覆盖损坏数据保留、隐藏和锁互斥；**尚未实机模拟创建中崩溃并完成恢复**。
- Linux 实机：通过 Bridge 连接仅绑定 `127.0.0.1` 的临时 26.3 离线服务器，进入后正常退出客户端。服务器白名单拒绝连接时观察到失败事件；临时服务端已停止。未验证远程服务器、正版账号、被踢出后的完整交互或 Realm。
- Bridge 初次资源加载画面显示真实 `ReloadInstance` 进度、完成阶段与百分比；Font/Language reload listener 确认中文资源就绪前使用 ASCII 英文，避免启动时 tofu。世界加载通过 Fabric `ScreenEvents` 叠加真实阶段与 `LevelLoadTracker` 进度；加载、登录/配置、首个区块等阶段通过 Bridge `Status` 消息回传启动器。启动器也会显示实例准备、Fabric 组件解析、游戏文件检查、组件校验和客户端启动阶段。原版仍拥有 tick、淡入和回调；没有人工截图、视觉、小窗口或资源重载验收，保存画面仍主要依赖原版。
- UI 声明增强体验最低要求 Minecraft 26.3（26.3+），并明确当前仅接入 26.3；普通模式的版本选择器列出 Mojang 清单中的全部正式版，关闭增强体验保留版本选择，普通下载控件和 `download::run(selected_version)` 路径可用。GUI 回归测试覆盖旧正式版留在列表且关闭开关后可下载。
- 最近验证：`DEEPSLATE_GRADLE_BIN=/tmp/deepslate-gradle/gradle-9.8.2/bin/gradle DEEPSLATE_GRADLE_OFFLINE=1 bridge/package.sh` 成功并同步 Rust 内嵌 JAR；`cargo test -p deepslate-core --lib -- --test-threads=1` 为 122 passed、6 ignored；`cargo test -p gui-iced --bin deepslate-iced` 为 22 passed。更早的 `cargo check --workspace` 通过；默认并行测试曾有两条地图库锁测试失败，串行重跑通过。仍有既有命名和 GUI warning，未进行全平台/全部规格验收。

## 2026-10-28 增强体验设置轮次

- 增强实例新增可选 JVM 配置：默认保留 JDK GC/堆策略；可选 ZGC、2–32 GB `-Xmx` 预设、紧凑对象头。ZGC 与紧凑对象头只在检测到 Java 25+ 时注入；不设置 `-Xms`。普通实例不读取这些配置。
- 增强实例新增启动器语言选择；启动时只更新增强实例 `options.txt` 的 `lang` 行，保留其他游戏设置。语言配置下一次启动生效。
- 新世界表单新增难度以及保留物品、生物破坏、昼夜循环、天气循环、敌对生物、PVP 六项规则。Bridge 在新建的集成服务端启动事件中通过 26.3 `GameRules` API 写入各维度；仅用于新建的单人世界。
- 增强模式快速加入区新增命名服务器收藏，可保存、选用、移除，最多保留 32 项；手动查询可显示 MOTD、版本、在线人数与延迟。查询目前使用直连 host/port，尚未做 DNS SRV 记录解析。
- 启动页在受管会话期间显示 Java 进程启动至 `world_ready` 的总时长，以及当前 Bridge 阶段时长。该值用于观察启动阶段，不是 CPU/磁盘 profiler。
- Bridge 因 CreateWorld 消息扩展升级至 `0.2.0` / IPC protocol `2`，旧组件代次会在下次安装流程中更新。该升级需联网解析 Fabric 组件；旧协议 Bridge 不作为新协议回退组合。
- 本轮源码编译验证：`cargo check --workspace` 成功；Bridge 使用本机 Gradle 9.8.2 离线构建成功，`bridge/package.sh` 已更新 `core/assets/deepslate-bridge-0.2.0.jar`，两份 JAR SHA-256 相同。未运行测试，也未进行本轮新增难度/规则/语言/服务器 ping 的 Minecraft 或网络实测；后续需用隔离世界检查规则写入与保存重进，检查语言、JVM 选项与在线服务器状态查询。

## 2026-10-28 好友首版

- 首版以只读方式把 MC 26.3 的好友快照缓存到 `<root>/config/friends/<uuid>.json`；启动器页面也曾说明关系操作仍交给 Mojang 好友服务。该实现已由下方 0.4.0 接管路径取代。

## 2026-10-28 启动器主导好友数据（Bridge 0.4.0）

- 保留 Minecraft 原生好友 Overlay、好友/申请 Tab、列表项、确认弹窗和按钮。`MinecraftMixin` 在 `MinecraftServicesDiscoveryService.createFriendsService` 调用点直接返回 Bridge 适配器，不创建 Mojang Authlib 好友服务对象，也不把登录 access token 传给它；Minecraft 登录、Xbox/XSTS/Minecraft 登录认证、多人服务器认证和屏蔽玩家 API 保持原样。
- 权威好友数据按账户 UUID 存在启动器 `<root>/config/friends/<uuid>.json`。启动器生成并保存 `DL1-<UUIDv4>` 好友码，好友页面可复制、添加好友码、接受/拒绝/撤回申请、删除好友和修改好友列表设置。游戏启动握手后接收快照；游戏原生 UI 发起的动作通过 Bridge 回到启动器验证、落盘，再把新快照推回游戏。
- Java `DeepslateFriendsService` 现在只是游戏进程内存镜像，不再自行生成或持久化好友数据。Bridge 同步按启动器账户 UUID 校验，游戏快照不再覆盖启动器存储。Presence 目前为空；好友码申请在联网发现接线前只会本机保存，尚未送达对方。
- Authlib 好友服务与 Presence 的远端调用被替代；联网发现、安全握手和真正跨设备送达仍未实现。
- Bridge 与内嵌资源当时仍为 0.4.0（尚未发布）。Fabric/Gradle 构建、`cargo check --workspace` 和 `git diff --check` 通过；Gradle 报告 `test NO-SOURCE`，没有运行测试。尚未启动 Minecraft 验证服务工厂重定向、Bridge 好友双向同步、中文资源和原生控件操作；这些需在隔离增强实例确认。

## 2026-10-29 去中心化好友网络（Bridge 0.5.0）

- 后端新增 `core/src/friendnet.rs`：每个账户 Ed25519 身份、DL2 好友码、直连 TCP、X25519 三轮握手、ChaCha20-Poly1305 加密帧、重连/心跳、共同好友单跳中继和 presence。设计、威胁模型与验收清单见 [DESIGN-friends-decentralized.md](./DESIGN-friends-decentralized.md)。
- `core/src/friends.rs` 扩展为权威关系状态机：`friends/incoming/outgoing` + `outbox` + `tombstones`；关系变更逐条签名，revision 高水位防重放，申请绑定目标公钥 `to`，Ack 只接受外发目标。旧 `DL1` 条目保留为本地记录，但联网操作要求 DL2。
- GUI 好友页显示网络监听端口、对外地址、在线好友数和每位好友的 presence；设置页新增「好友网络」端口/对外地址。网络随账户/设置启停，远端快照变化会通过 `sync_managed_friends` 回推当前 Bridge 会话。
- Bridge 0.5.0：`DeepslateFriendsService.presence(status)` 返回好友 presence，并通过新 `SelfPresence` 事件把游戏自身状态回传启动器；`presence_from_authlib` 映射到 `online/playing_* / offline`。游戏内添加好友输入上限从 64 提到 512，Java 只把公钥段转大写，保留大小写敏感的 base64url 地址段。
- 内嵌 JAR：`core/assets/deepslate-bridge-0.5.0.jar`，与 `bridge/build/libs/deepslate-bridge-0.5.0.jar` SHA-256 均为 `8c85c23e4763cc0bf5faf0d0df92a2847ddb3a9a983bf8ce7b4141695e7e3e35`；SHA-1 `1cdc01d221c1a59d69bead10ec6f7b6bebfbb350`。旧 0.4.0 JAR 保留在仓库中但不再嵌入；增强组件需在下次安装流程中更新到 0.5.0，否则旧 Bridge 不识别 DL2、也不会回传 presence。
- 验证：`./gradlew --offline build` 成功；`cargo check --workspace` 通过；`cargo test -p deepslate-core --lib -- --test-threads=1` 为 137 passed / 6 ignored；`cargo test -p gui-iced --bin deepslate-iced` 为 28 passed。新增测试覆盖加密握手、错误身份拒绝、签名篡改拒绝、presence/关系消息解析、真实监听端口上的「申请→Ack→接受→accept 外发→presence→删除」全链路、双账户申请/接受/presence/删除/重放状态机、错误目标公钥拒绝。
- **尚未实机验证**：两台机器直连/中继、26.3 原生好友 Overlay 的列表与 presence、游戏内长 DL2 码输入、启动器崩溃/断网重连、端口占用和根目录切换；Windows 已完成 IPC/ACL 代码与单元测试，但还没有 Java 25 + 26.3 + Bridge 的 Windows 实机证据。当前没有中心服务器；好友图直连没有 NAT 穿透，世界 P2P 隧道使用 Iroh 的 STUN/UPnP/Relay。中继只做单跳且能看到申请内容，显示名/UUID 未与 Mojang 正版账号做强绑定。

## 关键剩余工作（按规格，非完整验收结论）

1. **Windows 适配代码已落地，仍需实机验收。** `core/src/secure_fs.rs` 已为 Bridge 会话目录/文件、好友身份密钥和快照提供平台实现：Unix 保持 0700/0600，Windows 使用 owner-only DACL（拒绝继承）。`BridgeListener::bind` 的非 Unix 硬拒绝已移除；Windows 上 `bridge::tests` 的握手与受管会话测试通过。仍需在 Windows 上验证 Java 25 + Fabric 26.3 + Bridge 握手的真实游戏会话、会话清理、文件替换和游戏退出后持锁；没有 Windows + Minecraft 实机证据。
2. macOS 尚未实机验证；Unix 路径并不等于 macOS 验收。Linux 实机仍需覆盖暂停菜单中的实际“保存并退出”、资源重载、世界损坏/兼容提示、启动器意外退出与桥接中断等。
3. 增强模式 Realm 明确未实现。联机连接失败事件目前是通用错误；远程断线、被踢出、服务器资源包、正版验证及返回错误界面的完整行为需核实，避免凭本机离线服务器结果推断全部联机可用。
4. 缺少 GUI 视觉、560px 小窗口、资源加载／世界加载／保存提示及错误流程的实际验收。游戏内仍需完善世界加载与保存画面；等待画面、加载画面尚无截图验证。启动器进度是分阶段数据，事件总线仍需检查并发任务进度归属，避免显示其他任务的进度。
5. 自动安装与回退尚需加固：`fabric::ensure_base_version` 快速路径现在按本地版本 JSON 中的摘要核验客户端、适用的常规库、资源索引和资源对象；缺失或损坏时走下载修复。下载时也检查版本标识、客户端/库摘要格式、资源索引 SHA-1 及资源对象 SHA-1。该检查尚未在新的真实 26.3 安装/损坏缓存场景实测；基础下载与 Fabric profile 的原子写入、失败/中断、旧组合完整性和 Windows 原子替换仍需审计。组件代次复用、离线回退与 Bridge 自身升级/旧组合兼容也需验证。Fabric API 元数据约束解析目前只支持有限形式；新格式应安全处理并测试。
6. Java 25 是所需环境，当前 GUI 仍由用户选择已扫描的 Java，并非完整的自动 Java 安装/维护。验证缺失、版本不明、不同发行版和路径含特殊字符时的提示。
7. CLI 的版本、模组、游戏目录与增强开关尚未全面接入/验收；服务端、日志、options 和模式切换在运行中及重启后的表现需逐项审计。需要明确从普通实例迁入用户模组的入口，不自动混入。
8. 增强模式新世界表单现已包含难度与六项常用规则；仍需在隔离世界实测规则写入、保存重进和失败/取消回报。更多预设/数据包配置、创建中崩溃恢复以及恢复/删除互斥仍需验收。
9. Bridge 凭据与协议的权限、大小上限、重复请求、握手超时和异常连接需要跨平台审计；游戏因启动器崩溃或连接断开时不得被强杀。当前等待/退出闭环不证明所有错误、重连或重载场景。
10. 构建和交付：固定可重复的 Gradle wrapper 发行包摘要/依赖、自动检查 Bridge 源码与内嵌 JAR 同步，完善 CI、打包、协议兼容、升级/回退说明；重新跑全量测试和规格逐项验收。

## 本机验证与注意事项

```bash
cargo test -p deepslate-core --lib
cargo test -p gui-iced --bin deepslate-iced
cargo check --workspace
cd bridge && ./gradlew build
```

`./gradlew` 曾遇到 Java HTTPS 握手失败；本机曾通过系统 `curl` 下载 Gradle 到 `/tmp/deepslate-gradle/gradle-9.8.2`，并使用 `/tmp/deepslate-gradle/gradle-9.8.2/bin/gradle -p bridge build --offline` 构建。不要把 `/tmp` 工具或本机缓存作为交付依赖。

`target/enhanced-live/` 是包含用户世界**副本**与游戏资源副本的测试根；`target/enhanced-server-test/` 是临时本机服务器数据。它们属于本机测试产物，未提交；不要将其作为产品数据或删除用户原始世界。实机测试会打开 Minecraft 窗口，且会修改所指定测试世界：仅对隔离测试副本运行。`core/tests/enhanced_live.rs` 的 `DEEPSLATE_TEST_ROOT`、`DEEPSLATE_TEST_CREATE`、`DEEPSLATE_TEST_SERVER` 使用方式见源码；完整首次安装测试是 ignored，约需下载 26.3 所需资源。

继续前先检查 `git status`、运行必要测试并核实本文件是否仍与工作树一致；不要声称规格完成，直到逐项有相应平台和运行证据。
