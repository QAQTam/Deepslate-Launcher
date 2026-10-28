# Spec — 启动页重组与高级页合并

**日期**：2026-10-27\
**状态**：已被取代。其滚动/验收要求已落地（Launch/Friends/Settings 外层滚动，见 `gui-iced/src/views.rs:6`）；信息架构部分未实施。后续工作以 [SPEC-ui-optimization.md](./SPEC-ui-optimization.md)（2026-10-28，按含好友页的 8 项导航现状重写）为准。\
**范围**：`gui-iced` 前端布局/导航，不改 `core` 业务逻辑\
**前置**：[HANDOFF.md](./HANDOFF.md)

---

## 1. 目标

解决截图里最明显的三个问题：

1. 「启动」页信息密度过高，日常启动动作被安装/维护控件抢走；
2. 「高级」页只有两张卡，却占一个顶级导航；
3. 900×560 最小窗口下，启动页/设置页可能裁掉底部内容。

**成功标准**：在 900×560 和 1040×700 两个尺寸下，用户无需先找设置，就能完成“选版本/档案/Java → 启动游戏”；维护和高级参数仍可达，但默认收起。

---

## 2. 非目标

本 spec 明确不做：

- 不改 `core` 的下载、启动、服务端、地图、EULA 逻辑；
- 不改 EULA 模态和更新检查；
- 不引入新的 UI 组件库/依赖；
- 不重做配色、字体、圆角；
- 不做首页新闻、Modrinth/CurseForge、服务器插件等新功能；
- 不在本轮处理地图卡「更多」菜单、模组空状态跳转（见 §9 后续项）。

---

## 3. 目标信息架构

### 3.1 侧栏

从 7 项减为 6 项：

```text
首页
启动
世界
服务器
模组
设置
```

删除 `Page::Advanced` 及侧栏项。高级启动参数并入「启动」页。

### 3.2 「启动」页结构

```text
启动
├─ 开始游戏（主卡）
│  ├─ 版本 tab（最新正式版 / 已安装）+ 版本选择
│  ├─ 档案选择
│  ├─ Java 运行时选择
│  ├─ 状态行（档案/版本/游戏状态）
│  └─ [启动游戏] [下载当前版本]
├─ 快速加入服务器（次卡）
│  └─ host:port + [快速加入] + 最近地址
├─ 账户（折叠，默认收起）
│  ├─ 新建离线档案
│  └─ Microsoft 登录 / 回调 URL
├─ 安装与维护（折叠，默认收起）
│  ├─ Fabric 加载器安装
│  ├─ 刷新列表
│  └─ 重新扫描 Java
└─ 高级启动（折叠，默认收起）
   ├─ 游戏限制：禁用多人 / 禁用聊天
   └─ 窗口与图形：全屏 / 宽高 / 图形后端
```

**设计决定**：

- Fabric 安装留在「启动」页，但放进「安装与维护」折叠区，因为它是版本安装的一部分，不是全局设置。
- Java **选择**留在主卡；Java **重新扫描**放维护区。
- 档案选择留在主卡；“新建档案 / 微软登录”放「账户」折叠区，避免首次配置说明长期占据主卡。
- 高级参数放「高级启动」折叠区，不再有独立页面。
- 折叠区用 `App` 内 `bool` 控制，不做容器高度动画；打开/关闭直接重建视图。

### 3.3 设置页

保留现有全局配置，只做低风险整理：

- 常规：根目录、下载并发；
- 外观：明暗、日志窗口、标题品牌、中文默认值；
- 存储/诊断：地图库路径、实际路径；
- 「实际路径」卡片默认折叠，提供「打开配置目录」按钮；本轮保留可选择文本，不新增剪贴板按钮。

不在本轮把设置页拆成多个子页面。

---

## 4. 布局与响应式

### 4.1 启动页

主卡使用内部两行，避免 3 个 picker 挤在一行：

- 行 1：版本 tab + 版本 picker + Java picker；
- 行 2：档案 picker + 状态/按钮；
- 宽度不足时允许 picker 自然换行（iced 没有 CSS media query，使用 `row!` + `Length::Fill` 的弹性列）。

快速加入保持整行。

折叠区使用 `section_card`，标题行右侧放「展开/收起」按钮。

### 4.2 滚动

- `launch_page` 外层改为 `scrollable(...)`，高度 `Length::Fill`。
- `settings_page` 外层同样改为 `scrollable(...)`。
- 首页/世界/服务器/模组保持现状；服务器/地图内部已有 scrollable，不要套第二层。
- 验收时必须确认：900×560 下启动页可以滚动到「高级启动」和状态行，不出现裁切。

### 4.3 窗口最小尺寸

保持 `min_size: Size::new(900.0, 560.0)`，不为布局问题提高最小高度；用滚动解决。

---

## 5. 代码改动清单

### 5.1 状态

`gui-iced/src/main.rs` 的 `App` 新增：

```rust
show_launch_account: bool,       // 账户折叠区，默认 false
show_launch_maintenance: bool,   // 安装与维护，默认 false
show_launch_advanced: bool,      // 高级启动，默认 false
show_settings_paths: bool,       // 设置页实际路径，默认 false
```

初始化为 `false`。不持久化；重启后默认收起。

新增消息：

```rust
ToggleLaunchAccount,
ToggleLaunchMaintenance,
ToggleLaunchAdvanced,
ToggleSettingsPaths,
```

更新 `update()` 只翻转对应 bool，返回 `Task::none()`。

### 5.2 导航

- `Page` 删除 `Advanced`。
- `Page::label()` 删除对应分支。
- `view()` 侧栏删除 `nav(Page::Advanced)`。
- `page_view()` 删除 `Page::Advanced => advanced_page(...)`。
- 删除 `advanced_page()` 函数，把内容抽成 `launch_advanced_section(app)`。

### 5.3 启动页函数拆分

建议把当前 `launch_page()`（`gui-iced/src/main.rs:2565` 附近）拆成：

```rust
fn launch_page(app: &App, t: f32) -> Element<'_, Message>
fn launch_hero(app: &App) -> Element<'_, Message>
fn quick_join_card(app: &App) -> Element<'_, Message>
fn launch_account_section(app: &App) -> Element<'_, Message>
fn launch_maintenance_section(app: &App) -> Element<'_, Message>
fn launch_advanced_section(app: &App) -> Element<'_, Message>
```

原则：

- 复用现有控件和 `Message`；
- 不改版本选择、Fabric 安装、档案、Java 的所有 handler；
- 主卡只调整布局和顺序。

### 5.4 折叠区

统一模式：

```rust
let expanded = app.show_launch_maintenance;
let toggle = button(text(if expanded { "收起" } else { "展开" }))
    .style(style::secondary)
    .on_press(Message::ToggleLaunchMaintenance);
```

内容只在 `expanded` 时加入 `column![]`；不做 `Collapsible` 组件，不引入动画。

### 5.5 设置页

- `settings_page()`（`gui-iced/src/main.rs:3842` 附近）外层改 `scrollable`。
- 「实际路径」卡片改为折叠区，默认收起；由 `show_settings_paths` / `ToggleSettingsPaths` 控制。
- 暂不改设置字段的保存逻辑。

---

## 6. 验收标准

### 6.1 功能

- [ ] 侧栏只有 6 项，没有「高级」。
- [ ] 启动页首屏可见：版本 tab、版本 picker、档案 picker、Java picker、启动按钮。
- [ ] 「安装与维护」「账户」「高级启动」默认收起；展开后所有原有控件可用。
- [ ] 设置页「实际路径」默认收起，展开后路径/打开目录仍可用。
- [ ] Fabric 安装、刷新列表、档案创建、微软登录、Java 扫描、禁用多人/聊天、全屏/宽高/图形后端全部可达。
- [ ] 所有原有 `Message` 和 `AppConfig` 字段仍被使用；无核心逻辑变化。
- [ ] EULA gate / 更新检查 / CLI 行为不受影响。

### 6.2 布局

- [ ] 1040×700：启动页无重叠、无异常空白，主操作卡在视觉上优先。
- [ ] 900×560：启动页可滚动到最底部，按钮和折叠区不被裁切。
- [ ] 设置页在 900×560 可滚动到底部「保存设置 / 重置」。
- [ ] 世界/服务器/模组页未出现双滚动条或布局回归。

### 6.3 构建与测试

```bash
cargo test -q
# core: 70 passed, 4 ignored
# gui-iced: 3 passed（新增布局构造测试后数量可变）

cargo build
# exit 0
```

建议新增一个 `launch_page_constructs_at_small_size` 测试，只构造 view，不依赖真实窗口。

---

## 7. 实施顺序

1. 删除 `Page::Advanced` 导航，抽 `launch_advanced_section()`；先保持页面内容不变，确认没有消息/字段丢失。
2. 把启动页拆成 hero / quick join / account / maintenance / advanced。
3. 加入三个 `bool` 和 toggle 消息；默认收起。
4. 启动页外层加 `scrollable`；确认 560px 下不裁切。
5. 设置页外层加 `scrollable`；实际路径折叠。
6. 运行测试 + 两个尺寸人工验收。
7. 更新 `README.md` / `CLAUDE.md` 的页面结构描述。

---

## 8. 风险与回退

- **风险**：启动页卡片抽函数后，`Element` 生命周期/借用参数容易出错。优先保持函数签名 `fn xxx(app: &App) -> Element<'_, Message>`，不要在 helper 里 clone 大列表。
- **风险**：`scrollable` 套在已有滚动区域上会导致双滚动条。只给 Launch/Settings 外层加；Maps/Server/Mods 不动。
- **风险**：删除 `Page::Advanced` 后，如果其他代码/测试还引用会编译失败；先全局搜索 `Page::Advanced`。
- **回退**：本轮只改 `gui-iced/src/main.rs` 和文档；回退时用 `spy`/journal 针对单文件恢复，不要 `git checkout -- .`。

---

## 9. 后续项（本 spec 不做）

- 地图卡操作收进「更多」菜单；
- 模组空状态增加「去启动页选择 Fabric」；
- 首页右侧补最近世界/快捷入口；
- 提高 10–11px 辅助文字对比度和字号；
- 如确实需要 120fps，验证 `ICED_PRESENT_MODE=immediate`，但默认保持 vsync + 8ms tick。

---

## 10. 需要确认的两个产品选择

默认按本 spec 执行：

1. Fabric 安装放「启动 > 安装与维护」，不迁到设置；
2. 档案创建/微软登录放「启动 > 账户」折叠区，不迁到设置。

如果希望更彻底地精简启动页，可以把这两块都迁到设置页；那会改变本 spec 的 §3.2 和 §5.3，需先确认。
