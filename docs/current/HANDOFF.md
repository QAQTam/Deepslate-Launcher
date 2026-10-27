# Handoff — 2026-10-27

> 增强体验的最新状态与剩余工作见 [HANDOFF-enhanced-experience.md](./HANDOFF-enhanced-experience.md)。下文是较早的 EULA/UI 交接，不能代表增强体验当前状态。

**状态**：进行中，工作树未提交\
**项目**：Deepslate Launcher（Rust + iced 0.14）\
**下一份文档**：[SPEC-ui-restructure.md](./SPEC-ui-restructure.md)

---

## 1. 当前结论

这一轮主要完成了两件事：

1. **EULA 首次启动授权**：主窗口模态，官方正文抓取/缓存，滚动到底 + 勾选，`<root>/eula.txt` 标记。
2. **EULA 更新版本绑定**：接受时记录正文 SHA-1；启动时检查官方正文哈希，协议实质更新要求重新阅读并同意。

同时做了两个低风险 UI/性能修正：

- 动画 tick 从 `33ms`（约 30fps）改为 `8ms`（约 125fps 上限）；仍受显示器刷新率/wgpu vsync 限制，空闲不重绘。
- 世界页、服务端页中重复的 EULA 勾选改为状态文案；授权统一由启动/开服/建世界前的 EULA gate 处理。

---

## 2. 当前工作树状态

**没有提交**。工作树里包含本轮和之前多轮未提交的改动，`git status` 会看到 `core/src/*`、`gui-iced/src/main.rs`、`README.md`、`CLAUDE.md`、`Cargo.lock` 等大量变化；`core/src/eula.rs` 本身是未跟踪文件。

**不要**为了“清理工作树”执行 `git checkout -- .` 或 `git checkout -- core/src/events.rs`。之前的会话里 `git checkout` 曾误删过 `events.rs` 的 `ServerLog`/`ServerState`，也影响过 `config.rs`。需要回退请用 `spy` 或针对单个 `write/edit` 的 journal，不要整文件 checkout。

用户提供的 UI 截图位于：

```text
/home/qaqtamsy/Pictures/屏幕截图/
```

当前已看过的页面：首页、启动、世界、服务器、模组、高级、设置。

---

## 3. EULA 实现

### 3.1 文件与路径

- `core/src/eula.rs`：抓取、解析、缓存、接受标记、更新检查。
- `<root>/eula.txt`：服务端兼容的 `eula=true`。
- `<root>/legal/eula.txt`：提取后的正文。
- `<root>/legal/eula.html`：原始 HTML。
- `<root>/legal/eula.json`：抓取元数据（URL / fetched_at / SHA-1）。
- `<root>/legal/eula-accepted.json`：用户接受时绑定的正文 SHA-1（GUI 写入）。

### 3.2 抓取与解析

- URL：`https://www.minecraft.net/en-us/eula`（`/eula` 会 302）。
- 使用独立 HTTP/1.1 客户端 + 干净 UA。原因：
  - Akamai 对 HTTP/2 返回 `INTERNAL_ERROR`；
  - UA 中带 `+https://...` 会被卡住/超时。
- `scraper` 只取 `#main-content`，保留标题、段落、列表、链接文字和标点；只折叠 HTML 缩进空白。
- 校验：
  - 首行标题；
  - `COMPANY INFORMATION` 段；
  - 正文至少 5000 字符；
  - 缓存文本 SHA-1 与元数据不一致时拒绝缓存。
- 2026-10-27 实测抓取：**16,754 字符 / 160 行**，首行 `Minecraft End(er)-User License Agreement (“EULA”)`，末行为巴西公司地址。

### 3.3 接受与更新

- GUI 同意：`accept_document(config, sha1)`，先写 `eula-accepted.json`，再写 `eula.txt`。
- 启动时：`check_for_update(config)`：
  - 未接受 → `NotAccepted`；
  - 哈希一致 → `Unchanged`；
  - 哈希变化 → `Changed(document)`，GUI 立即失效旧同意、打开新正文模态，必须重新滚动 + 勾选。
- `eula_is_accepted()` 同时检查：
  - `eula.txt` 为 `true`；
  - 接受哈希与当前缓存正文哈希一致（有缓存时）。
- 网络失败：
  - 不把“抓不到”当成“未更新”；
  - GUI 提示“无法检查 EULA 更新，将按已接受版本继续”；
  - 若接受哈希与缓存正文已经不一致，仍然要求重新确认。
- CLI：
  - `--launch` / `--host-server` / `--create-world` / `--generate-map` **每次**都要显式 `--accept-eula`；
  - 不依赖旧 `eula.txt` 静默放行；
  - `accept()` 只写 marker，不覆盖已有版本绑定。

### 3.4 GUI

- 主窗口 `stack` + `opaque` 全窗遮罩，不是独立日志窗口。
- `Scrollable` + `on_scroll` 判断正文是否到底。
- 未到底或未勾选时按钮不可用。
- 弹窗关闭后继续被拦截的 `PendingAction`（启动 / 服务端安装 / 服务端启动 / 建世界）。
- 首次下载完成后预取 EULA，下一次启动时可直接展示。
- 世界/服务端页已移除重复 EULA checkbox，改为状态文案。

关键位置：

- `core/src/eula.rs:25` URL
- `core/src/eula.rs:81` `EulaUpdate`
- `core/src/eula.rs:113` `accept_document`
- `core/src/eula.rs:146` `accepted_sha1`
- `core/src/eula.rs:202` `check_for_update`
- `gui-iced/src/main.rs:1794` `eula_is_accepted`
- `gui-iced/src/main.rs:1813` `handle_eula_update`
- `gui-iced/src/main.rs:2350` `eula_modal`
- `gui-iced/src/main.rs:4042` CLI gate

---

## 4. UI 现状与已知问题

截图里的视觉体系是成立的：牛皮纸黄 + 石板灰、卡片圆角、侧栏 active 态一致。当前问题主要是信息架构和空间利用。

### 4.1 启动页

`gui-iced/src/main.rs:2565` 的 `launch_page` 是 6 张卡片的双栏 masonry：

- 左：版本 / Fabric / 档案
- 右：Java / 启动 / 快速加入

问题：

- Java 卡片因路径换行被撑高，右侧出现大块空白；
- 启动操作卡反而不是视觉主卡；
- Fabric 安装、档案登录等“维护动作”与日常启动抢焦点；
- 页面注释明确写着“整页不滚动”（`:2869`）。

### 4.2 高级页

`gui-iced/src/main.rs:3753` 的 `advanced_page` 只有两张卡：

- 游戏限制；
- 窗口与图形。

页面大半为空，却占一个顶级侧栏项。建议合并到启动页的「高级启动」折叠区，或设置页的「游戏与启动」分区。

### 4.3 设置页

`gui-iced/src/main.rs:3842` 的 `settings_page`：

- 「绝对路径」卡片占面积大，属于低频诊断信息；
- 外观区有多条长标签换行；
- 2×2 网格同样是“整页不滚动”（`:3969`）。

### 4.4 其他

- 首页「当前状态」只占左半边，右侧空着。
- 世界地图卡右侧 7 个按钮过密，建议「进入」+「更多」。
- 模组空状态有红色提示，但缺少直接跳转「启动页选择 Fabric」的按钮。
- 窗口最小高度为 560px；启动页/设置页当前可能裁掉底部内容。

---

## 5. 帧率结论

之前不是错觉：`subscription` 里是 `Duration::from_millis(33)`，约 30fps，但注释写的是 16ms。

现在：

```rust
// gui-iced/src/main.rs:2249
iced::time::every(Duration::from_millis(8))
```

行为：

- 只在 `page_anim.is_animating()` 时订阅；
- 300ms 动画结束后自动停止；
- 空闲不重绘，FPS 工具显示 0 是正常的；
- 8ms 只是上限；实际仍由 wgpu `AutoVsync` 和显示器刷新率决定：
  - 60Hz → 最高 60fps；
  - 120Hz → 才有机会到 120fps；
  - 想临时验证无 vsync，可用 `ICED_PRESENT_MODE=immediate`，不建议作为默认。

性能测试（800 个版本）中启动页 view 构造约 **323µs/帧**，8ms 预算足够，不需要为了 120fps 担心 UI 线程。

---

## 6. 验证状态

```bash
cargo test -q
# core: 70 passed, 4 ignored
# gui-iced: 3 passed

cargo build
# exit 0
```

联网 EULA 测试（ignored）：

```bash
cargo test -p deepslate-core eula::tests::fetches_live_official_eula -- --ignored
cargo test -p deepslate-core eula::tests::live_check_detects_accepted_hash_mismatch -- --ignored
```

两项均通过。

CLI 探针：

- 无 `--accept-eula`：exit 2；
- 带 `--accept-eula`：写入 `<root>/eula.txt`，继续到预期的“客户端不存在”错误；
- 第二次无 flag：仍然拒绝（CLI 必须每次显式同意）。

**未验证**：GUI 实际视觉/滚动/布局。当前环境没有 `DISPLAY`/`WAYLAND_DISPLAY`，无法启动 iced 窗口做截图验收。

---

## 7. 已知限制 / 风险

1. **工作树未提交**，且包含大量历史未提交改动；不要整文件回滚。
2. **GUI 视觉未验收**：EULA 模态 820px 卡片、560px 高度、滚动到底检测都只做了编译/构造测试。
3. **EULA 更新检查只在应用启动时做**：启动器长期运行期间协议更新，要等下次启动才会发现。
4. **离线策略**：检查失败时允许已接受版本继续；如果要求断网也硬阻止，需要另加设置/策略。
5. **CLI 无版本绑定**：`--accept-eula` 只写 marker；GUI 下次检查会迁移/校准哈希。CLI 每次必须显式 flag。
6. **启动页/设置页 560px 高度可能裁切**，见 spec。
7. **`docs/superpowers/` 下的旧 React/Void Crystal 文档已过期**，不要按其技术栈实现；以本目录和 CLAUDE.md 为准。

---

## 8. 下一步

见 [SPEC-ui-restructure.md](./SPEC-ui-restructure.md)：先做启动页重组和高级页合并，不改核心逻辑，目标是减少启动页认知负担、移除一个顶级导航，并解决 560px 高度下的裁切。
