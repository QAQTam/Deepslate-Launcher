# Spec — UI 体验优化（第二轮）

**日期**：2026-10-28\
**状态**：待实施\
**范围**：`gui-iced` 前端布局/导航/交互细节；除特别标注外不改 `core` 业务逻辑，不涉及 `bridge`\
**取代**：[SPEC-ui-restructure.md](./SPEC-ui-restructure.md)（其滚动与验收部分已落地，信息架构部分未实施，本文按当前代码现状重写）\
**前置**：[HANDOFF.md](./HANDOFF.md)、[HANDOFF-enhanced-experience.md](./HANDOFF-enhanced-experience.md)、[AUDIT-2026-10-27.md](./AUDIT-2026-10-27.md)

---

## 1. 文档与代码现状核对（2026-10-28）

写本文前逐项对照了 docs 与工作树代码，结论如下。

### 1.1 审计与旧 spec 中已经修复、无需再做的项

| 来源 | 项 | 代码证据 |
| --- | --- | --- |
| AUDIT C02 | 主按钮白字对比度不足 | `gui-iced/src/style.rs:179` 主按钮改为深色文字 `0x15171a`；`style.rs:375-384` 有 ≥4.5:1 断言测试 |
| AUDIT C01 | 900×560 启动/设置页裁切、不可滚动 | `gui-iced/src/views.rs:6-8` `page_requires_outer_scroll` 对 Launch/Friends/Settings 包外层 `scrollable`；`main.rs:3241` 有构造测试 |
| AUDIT D01 | EULA 模态无取消出口 | `views.rs:190-193` 「取消」按钮 + `Message::EulaCancel`（`main.rs:1834`） |
| AUDIT D04 | 回收站一键永久清空 | `views.rs:970-979` 「清空回收站」→「确认永久清空？」两段式确认 |
| HANDOFF §4 | 世界/服务端页重复 EULA 勾选 | 已改为状态文案（HANDOFF 记录） |
| 性能 | 33ms tick / 版本列表逐帧克隆 | 8ms 订阅 + `filtered` 缓存（HANDOFF §5、提交 1c50608） |

### 1.2 仍未实施、构成本轮主体的项

1. **SPEC-ui-restructure 的信息架构重组完全未做**：`Page::Advanced` 仍是顶级导航（`views.rs:2007` `advanced_page` 只有两张卡），启动页仍是双栏卡片瀑布（`views.rs:719-751`），无任何折叠区状态（无 `show_launch_*` bool、无 `ToggleLaunch*` 消息）。
2. **侧栏从 spec 写作时的 7 项膨胀到 8 项**：Home、Launch、Friends、Maps、Server、Mods、Advanced、Settings（`main.rs:62-71`）。好友页是增强体验新增的，旧 spec 按没有好友页的情况写了 7→6，需按 8→7 重算。
3. **世界卡一行 8 个次级按钮**：进入/重命名/复制/导出/导出为.../备份/恢复/删除（`views.rs:1032-1104`）；且 busy 状态只禁用「进入」，其余操作无 `game_running` 防护（审计 A06 的 UI 侧部分仍开放）。
4. **辅助文字仍大量使用 10–11px**（如 `views.rs:611,647-658,667,759,880`），对比度已修但字号过小的问题（AUDIT C02 后半句、SPEC §9）未处理。
5. **模组空状态无跳转**：`views.rs:1476`「mods/ 里还没有模组」没有「去启动页安装 Fabric」按钮（SPEC §9 遗留项）。
6. **560px 视觉验收缺口扩大**：增强体验轮次新增了好友页、增强设置、快速加入收藏等界面，从未做过视觉/小窗验收（HANDOFF-enhanced 剩余工作第 4 条）。

### 1.3 判定为过期、不再列入计划的项

- HANDOFF §4.4「首页右侧空置」：首页已改为单列（状态卡 + 继续游戏卡 + 启动按钮行），不再是左右分栏空半边（`views.rs:924-938`）。
- AUDIT D05 中「首页/高级页卡片宽度收缩、两列空间浪费」：首页已重排；高级页问题由本文合并方案解决。

---

## 2. 目标

1. 删除「高级」顶级导航，侧栏 8 → 7 项；高级启动参数并入启动页折叠区。
2. 启动页按使用频率分层：选版本/档案/Java + 启动为主卡首屏；账户、安装维护、高级参数默认收起。
3. 世界卡操作收敛为「进入 + 更多」，并把游戏运行期间的占用防护补齐到所有破坏性/写操作（UI 侧）。
4. 提升小字号可读性：辅助文字下限 12px，正文 13px。
5. 建立可重复的小窗视觉验收手段（离屏渲染），覆盖本轮及增强体验新增页面。

**成功标准**：900×560 与 1040×700 下，用户不展开任何折叠区即可完成「选版本/档案/Java → 启动游戏」；世界页日常操作只需一次点击 + 一次确认；全部页面在离屏渲染验收中无裁切、无遮挡。

## 3. 非目标

- 不改 `core` 下载/启动/服务端/地图/EULA 逻辑；不引入新 UI 依赖；
- 不重做配色体系、圆角、字体族；不改 bridge/游戏内画面（增强体验阶段 4 单独推进）;
- 事件进度归属（审计 A12 / HANDOFF-enhanced 第 4 条）需要 core 事件带 operation id，属 core 改动，列入 §10 后续项；
- 不做首页新闻、Modrinth/CurseForge 浏览等新功能。

---

## 4. 信息架构

### 4.1 侧栏（8 → 7）

```text
首页
启动
好友          （增强体验新增，保留顶级）
世界
服务器
模组
设置
```

删除 `Page::Advanced`：`main.rs:62-71` 枚举、`label()` 分支、侧栏 `nav(Page::Advanced)`、`page_view()` 分支、`views.rs:2007-2092` `advanced_page()` 整体删除。内容迁入启动页「高级启动」折叠区（§5.4）。删除前全局搜索 `Page::Advanced` 确认无残留引用（含测试）。

### 4.2 启动页结构

```text
启动
├─ 开始游戏（主卡 hero，首屏完整可见）
│  ├─ 行 1：版本 tab + 版本 picker + Java picker
│  ├─ 行 2：档案 picker + 状态摘要
│  ├─ 进度区（下载/Fabric/受管会话阶段，现状保留）
│  └─ [启动游戏] [下载版本]（增强模式隐藏下载，显示增强状态行）
├─ 快速加入服务器（次卡）
│  ├─ host:port + [快速加入]
│  ├─ 收藏列表（增强模式，日常可达，行内：地址按钮 + 状态）
│  └─ 「管理收藏」展开行：命名输入 + [收藏地址] + [查询]/[移除]
├─ 账户（折叠，默认收起）
│  └─ 新建离线档案 / Microsoft 登录 / 回调 URL
├─ 安装与维护（折叠，默认收起）
│  ├─ Fabric 安装（基准版本 + 加载器 + [安装 Fabric] + [刷新列表]）
│  └─ [重新扫描 Java]
└─ 高级启动（折叠，默认收起）
   ├─ 游戏限制：禁用多人 / 禁用聊天
   └─ 窗口与图形：全屏 / 宽高 / 图形后端
```

设计决定（沿用 SPEC-ui-restructure §10 的默认选择，两点均按原默认执行）：

1. Fabric 安装留在启动页「安装与维护」折叠区，不迁设置页；
2. 档案创建/微软登录留在启动页「账户」折叠区，不迁设置页。

新增决定：

3. 增强模式下主卡版本行显示固定 26.3 状态文案（现 `views.rs:433-440` 已有该逻辑，保留），版本 tab/picker 隐藏；「增强体验」说明卡（`views.rs:725-732`）并入主卡状态区，不再单独占一卡。
4. 快速加入的收藏列表是日常入口，保持展开；仅把低频的「命名 + 收藏 + 查询 + 移除」管理操作收进「管理收藏」展开行。
5. 折叠区用 `App` 内 `bool` 控制，不做动画，打开/关闭直接重建视图。

---

## 5. 代码改动清单

### 5.1 状态与消息（`gui-iced/src/main.rs`）

```rust
// App 新增字段，均初始化 false，不持久化
show_launch_account: bool,
show_launch_maintenance: bool,
show_launch_advanced: bool,
show_manage_favorites: bool,      // 快速加入「管理收藏」展开行
map_menu_open: Option<String>,    // 世界卡「更多」展开，存 map id

// 新增消息
ToggleLaunchAccount,
ToggleLaunchMaintenance,
ToggleLaunchAdvanced,
ToggleManageFavorites,
ToggleMapMenu(String),
```

`update()` 对应分支只翻转 bool / 设置 Option，返回 `Task::none()`。

### 5.2 启动页函数拆分（`gui-iced/src/views.rs:314` `launch_page`）

```rust
fn launch_page(app: &App, t: f32) -> Element<'_, Message>
fn launch_hero(app: &App) -> Element<'_, Message>
fn quick_join_card(app: &App) -> Element<'_, Message>
fn launch_account_section(app: &App) -> Element<'_, Message>     // 折叠
fn launch_maintenance_section(app: &App) -> Element<'_, Message> // 折叠
fn launch_advanced_section(app: &App) -> Element<'_, Message>    // 折叠，原 advanced_page 内容
fn collapsible_section(标题, expanded, toggle_msg, 内容) -> ...   // 统一折叠头模式
```

原则：

- 复用现有控件与 `Message`；版本选择、Fabric 安装、档案、Java 的全部 handler 不动；
- 函数签名统一 `fn xxx(app: &App) -> Element<'_, Message>`，helper 内不 clone 大列表；
- 主卡只调整布局与顺序，双栏瀑布（`views.rs:719-751`）改为单列：hero → 快速加入 → 三个折叠区。

折叠区统一模式：

```rust
let expanded = app.show_launch_maintenance;
// 标题行右侧「展开/收起」按钮，style::secondary，on_press(Message::ToggleLaunchMaintenance)
// 内容仅在 expanded 时加入 column![]
```

### 5.3 世界卡「进入 + 更多」（`views.rs:1032-1104`）

- 行 1：世界名 + 徽章 + [进入]（或受管会话的保存退出态）+ [更多]；
- `map_menu_open == Some(world.id)` 时追加行 2：重命名/复制/导出/导出为.../备份/恢复/删除，恢复与删除维持现有 danger + 二次确认文案；
- 「更多」按钮即 `ToggleMapMenu(world.id)`；打开另一张卡的菜单时自动收起前一张；
- **占用防护（审计 A06 的 UI 侧）**：`busy = game_running || launching` 时，行 2 全部按钮与「更多」一样置为不可点（当前只有「进入」做了 busy 禁用）。core 层的任务互斥/地图锁仍按 AUDIT A06 建议另行处理，见 §10。

### 5.4 字号规范

- 辅助/提示文字：下限 **12px**（现 10–11px 的 `recent_servers`、`saved_server_profiles`、页首路径、徽章等逐处上调）；
- 正文与列表项：13px；页面标题 28px 不变；
- 徽章（极限/作弊等）最低 11px，且只用 `text_muted` 及以上对比度；
- 本次只调字号，不改颜色值——对比度已由 `style.rs` 测试守护。

### 5.5 模组空状态与首页微调

- 模组空状态（`views.rs:1476`）追加 `[去启动页安装 Fabric]` 按钮，`on_press(Message::Navigate(Page::Launch))`；
- 首页保持单列现状，仅把提示行（`views.rs:930-932`）与状态行合并排版，减少两行 12px 文字的堆叠；不做右侧栏新功能。

### 5.6 设置页

外层滚动已就位（§1.1），本轮不再改动设置页结构。旧 spec 的「实际路径折叠」搁置：设置页在 560px 下经外层滚动已可达底部按钮，该项收益低。

---

## 6. 布局与滚动约束

- 启动页外层 `scrollable` 已存在（`page_requires_outer_scroll` 包含 Launch），主卡必须保证 900×560 首屏完整：hero 高度按 4 行内容估算，超出即说明行距/折叠头尺寸过大；
- 快速加入与折叠区允许滚动到达；折叠区展开时页面自然变长，依赖外层滚动，不要给折叠区内部再套 `scrollable`；
- 世界/服务器/模组页维持各自内部滚动，不纳入外层滚动（避免双滚动条，沿用现 `page_requires_outer_scroll` 白名单）；
- 最小窗口保持 `900×560`，不为布局问题提高最小高度。

---

## 7. 视觉验收（本轮新增手段）

审计已验证 tiny-skia 离屏渲染可行（AUDIT §三 C01 使用过临时探针）。本轮将其固化为可重复工具：

1. 在 `gui-iced` 增加 `--screenshot <page> <width>x<height> <out.png>` 调试 flag（或 ignored 测试 + dev-dependency），用 iced tiny-skia 软件后端渲染 `view(&app)` 输出 PNG；
2. 覆盖矩阵：{Launch, Friends, Maps, Server, Mods, Home, Settings} × {900×560, 1040×700}，折叠区加测「全部展开」状态；
3. 验收标准（对照渲染图逐张检查）：主卡五个要素（版本 tab/picker、档案、Java、状态、启动按钮）在 900×560 首屏可见；无按钮被裁成细线；世界卡行 2 展开后不遮挡其他卡；
4. 渲染检查是布局验收，不能替代实机交互测试；有显示环境时补 900×560 手工过一遍启动→进入世界流程。

注意：构造测试（现有 `launch_and_settings_views_expose_scrollable_content` 等）继续保留并随新折叠区扩展，但「调用了 view」不等于「渲染无裁切」，两者都要。

---

## 8. 验收标准

### 8.1 功能

- [ ] 侧栏 7 项，无「高级」；全局无 `Page::Advanced` 引用。
- [ ] 启动页首屏（900×560）可见：版本选择（或增强状态）、档案、Java、启动按钮。
- [ ] 账户/安装与维护/高级启动/管理收藏默认收起；展开后原控件全部可用：Fabric 安装、刷新列表、Java 重扫、离线档案、微软登录、禁用多人/聊天、全屏/宽高/图形后端、收藏增删查。
- [ ] 世界卡默认只有 进入 + 更多；更多展开含全部 7 个次级操作；恢复/删除保留二次确认。
- [ ] 游戏运行/启动中，世界卡全部写操作与更多菜单不可点。
- [ ] 模组空状态可一键跳转启动页。
- [ ] 全部原有 `Message` 与 `AppConfig` 字段仍被使用；EULA gate、更新检查、CLI、增强体验流程不受影响。

### 8.2 布局

- [ ] §7 渲染矩阵全部通过：无裁切、无遮挡、无双滚动条。
- [ ] 辅助文字 ≥12px；无 10px 正文残留。

### 8.3 构建与测试

```bash
cargo test -p gui-iced --bin deepslate-iced
cargo test -p deepslate-core --lib -- --test-threads=1
cargo check --workspace
cargo fmt --all -- --check
```

新增至少：折叠区构造测试（含全部展开）、`Page::Advanced` 删除后的页面分发测试、世界卡菜单开关状态机测试。

---

## 9. 实施顺序

1. **删高级页**：抽 `launch_advanced_section()` 并入启动页（先保持原布局塞入折叠区，删 `Page::Advanced`，编译+测试通过）。
2. **启动页拆分**：hero / quick join / account / maintenance 函数化；引入 4 个 bool + toggle 消息；双栏改单列。
3. **世界卡**：`map_menu_open` + 更多菜单；busy 全操作禁用。
4. **细节**：字号规范逐处上调；模组空状态跳转；首页提示行合并。
5. **验收**：离屏渲染工具 + 全矩阵跑图；`cargo test` 全绿；人工 900×560 与 1040×700 过一遍核心流程。
6. **文档**：更新 `README.md`、`CLAUDE.md` 页面结构描述与本文件状态；`SPEC-ui-restructure.md` 标记已被本文取代。

每步独立可编译、可回退。

## 10. 风险与后续项

风险：

- 拆函数后 `Element` 借用/生命周期报错——签名统一 `&App`，必要时先 `clone` 小字段而不是整列表；
- 世界页/服务页已有内部滚动，误加外层滚动会出现双滚动条——只动 `page_requires_outer_scroll` 白名单内的页；
- 工作树包含大量未提交改动，回退只用 spy/journal 单文件恢复，禁止 `git checkout -- .`（HANDOFF §2 明确警告过）。

后续项（本轮不做，登记在案）：

1. 地图操作 core 层互斥与占用检测（AUDIT A06 完整修复）；
2. 事件进度带 operation id，修复并发任务进度串扰（AUDIT A12 / HANDOFF-enhanced 第 4 条）；
3. 增强体验阶段 4 的游戏内加载/等待/保存画面与启动器阶段进度联动（SPEC-enhanced-experience §8）；
4. 服务器查询 DNS SRV 解析（HANDOFF-enhanced 2026-10-28 轮记录）；
5. 若需 120fps，再验证 `ICED_PRESENT_MODE=immediate`，默认保持 vsync。
