# Obsidian Launcher - 破坏性设计文档

**日期**: 2026-04-26
**状态**: 已批准
**版本**: 1.0

---

## 1. 设计概述

对 Obsidian Launcher 前端进行完全重新设计，采用 **Void Crystal** 美学，打造具有沉浸感的游戏启动器界面。

### 设计目标

- 突破现有扁平化设计的局限
- 提供电影级动效体验
- 保持功能性与美观性的平衡
- 引入 Minecraft 新闻中心功能

---

## 2. 设计语言

### 2.1 色彩系统 (Void Crystal Palette)

```css
/* 背景层次 */
--bg-deep: #0f0d1a;        /* 最深背景 */
--bg-primary: #1e1b4b;     /* 主背景 */
--bg-elevated: #312e81;    /* 提升元素 */
--bg-card: #4c1d95;        /* 卡片背景 */

/* 强调色 */
--accent-primary: #8b5cf6;  /* 主强调 - 紫色 */
--accent-glow: rgba(139, 92, 246, 0.4);
--accent-secondary: #6366f1; /* 次强调 - 靛蓝 */
--accent-hot: #a78bfa;       /* 悬停态 */

/* 发光效果 */
--glow-purple: 0 0 20px rgba(139, 92, 246, 0.5);
--glow-border: 0 0 0 1px rgba(139, 92, 246, 0.3);

/* 文字层次 */
--text-primary: #f5f3ff;    /* 主文字 */
--text-secondary: #c4b5fd;  /* 次文字 */
--text-muted: #8b5cf6;       /* 弱文字/标签 */
```

### 2.2 字体系统

```css
--font-display: 'Outfit', sans-serif;  /* 标题 */
--font-mono: 'JetBrains Mono', monospace;  /* 代码/数字 */
--font-body: system-ui, -apple-system, sans-serif;  /* 正文 */
```

### 2.3 空间系统

```css
--space-xs: 4px;
--space-sm: 8px;
--space-md: 16px;
--space-lg: 24px;
--space-xl: 32px;
--space-2xl: 48px;

--radius-sm: 8px;
--radius-md: 12px;
--radius-lg: 16px;
--radius-xl: 24px;
```

### 2.4 动效规范

**工具选择**: Framer Motion (主), CSS 动画 (辅助)

**动画类型**:

| 场景 | 动画类型 | 参数 |
|------|----------|------|
| 页面切换 | Stagger Children | delay: 0.05s, staggerChildren: 0.1 |
| 卡片悬停 | Scale + Glow | scale: 1.02, shadow: glow-purple |
| 按钮按压 | Scale Down | scale: 0.98 |
| 侧边栏折叠 | Width Transition | spring: stiffness: 300, damping: 30 |
| Modal 入场 | Scale + Fade | spring: stiffness: 200, damping: 25 |

**性能规则**:
- 仅使用 GPU 属性: transform, opacity, filter
- 禁止动画: width, height, top, left, font-size
- 尊重 prefers-reduced-motion

---

## 3. 布局架构

### 3.1 整体结构

```
┌─────────────────────────────────────────────────────┐
│ [Sidebar]  │  [Main Content Area]                   │
│            │                                        │
│  Logo      │  ┌──────────────────────────────────┐  │
│  ─────     │  │  Page Header                     │  │
│  首页      │  └──────────────────────────────────┘  │
│  启动      │                                        │
│  设置      │  ┌──────────────────────────────────┐  │
│            │  │                                  │  │
│  ─────     │  │  Content Area                    │  │
│  新闻      │  │  (scrollable)                    │  │
│            │  │                                  │  │
│  ─────     │  └──────────────────────────────────┘  │
│  状态      │                                        │
└─────────────────────────────────────────────────────┘
```

### 3.2 侧边栏规范

**展开态**:
- 宽度: 220px
- 显示: Logo + 图标 + 文字标签
- 过渡: spring 动画

**折叠态**:
- 宽度: 64px
- 显示: Logo (小) + 图标
- Tooltip 提示文字

**状态持久化**: 保存到 localStorage

### 3.3 页面结构

**首页 (Home)**:
- 快速状态卡片 (当前版本/档案/游戏状态)
- Minecraft 新闻卡片区 (嵌入式)
- 最近活动

**启动 (Launch)**:
- 版本选择器 (Tabs: 正式版/测试版/历史版)
- 档案选择
- Java 运行时
- 并发数设置
- 下载进度 (可视化)
- 启动按钮 (大,醒目)

**设置 (Settings)**:
- 目录配置
- 版本隔离开关
- 存档隔离开关
- UI 模式切换 (gorgeous/performance)
- 清空游戏文件
- 重置配置

**新闻 (News)** - 独立页面:
- 标题区
- 新闻卡片流
- 分类筛选
- 外部链接到 minecraft.net

---

## 4. 组件设计

### 4.1 导航组件

**SidebarNav**:
- 可折叠侧边栏
- Active 状态发光边框
- Hover 态 scale + 背景高亮

**TabBar** (版本选择等):
- 下划线指示器动画
- 标签间 smooth 过渡

### 4.2 卡片组件

**GlassCard**:
```css
background: rgba(76, 29, 149, 0.3);
backdrop-filter: blur(12px);
border: 1px solid rgba(139, 92, 246, 0.2);
border-radius: var(--radius-lg);
box-shadow: var(--glow-border);
```

**Hover 态**:
- scale: 1.02
- box-shadow: var(--glow-purple)
- border-color: rgba(139, 92, 246, 0.5)

### 4.3 按钮组件

**PrimaryButton**:
- 背景: linear-gradient(135deg, var(--accent-primary), var(--accent-secondary))
- 发光阴影
- Hover: 增强 glow
- Active: scale(0.98)

**SecondaryButton**:
- 透明背景 + 边框
- Hover: 背景填充

**DangerButton**:
- 红色调 (#ef4444)
- 确认对话框

### 4.4 表单组件

**Input**:
- 深色背景 + 紫色边框
- Focus: glow 效果
- Error: 红色边框 + 错误文字

**Select**:
- 自定义下拉箭头 (SVG)
- 选项列表发光效果

**Checkbox/Toggle**:
- 紫色开启态
- 平滑过渡动画

### 4.5 进度指示

**DownloadProgress**:
- 条形进度条 + 百分比
- 当前文件名称
- 流光动画效果
- 粒子消散效果 (gorgeous 模式)

### 4.6 Modal 组件

**Modal**:
- 玻璃拟态背景
- 缩放 + 淡入动画
- 点击外部关闭
- ESC 键关闭
- Focus trap

---

## 5. 功能模块

### 5.1 首页 (Home)

**组件**:
- `HomePage`
- `StatusCard` - 显示当前状态
- `NewsCard` - 新闻摘要卡片
- `QuickActions` - 快速操作入口

### 5.2 启动页 (Launch)

**组件**:
- `LaunchPage`
- `VersionSelector` - 版本选择器 (带 tabs)
- `ProfileCard` - 档案选择卡片
- `JavaSelector` - Java 运行时选择
- `ConcurrencyControl` - 并发数设置
- `DownloadProgress` - 下载进度可视化
- `LaunchButton` - 启动按钮

### 5.3 设置页 (Settings)

**组件**:
- `SettingsPage`
- `DirectorySection` - 目录配置
- `IsolationSettings` - 隔离设置
- `UiModeToggle` - 界面模式切换
- `DangerZone` - 危险操作区

### 5.4 新闻页 (News)

**组件**:
- `NewsPage`
- `NewsHeader` - 页面标题
- `NewsFilter` - 分类筛选
- `NewsCard` - 新闻条目卡片
- `NewsDetail` - 新闻详情 (Modal)

**数据源**:
- 主: minecraft.net 官方 RSS/API
- 次: 可折叠的 CurseForge/Twitter RSS

---

## 6. 状态管理

### 6.1 Context 拆分

原 `useAppState` (550+ 行) 拆分为:

```
 contexts/
  ├── AppProvider.tsx      # 根 provider
  ├── VersionContext.tsx   # 版本相关状态
  ├── ProfileContext.tsx   # 档案状态
  ├── DownloadContext.tsx  # 下载/进度状态
  ├── ConfigContext.tsx    # 配置状态
  └── GameContext.tsx      # 游戏运行状态
```

### 6.2 自定义 Hooks

```typescript
hooks/
  ├── useVersion.ts        # 版本列表/选择逻辑
  ├── useProfile.ts        # 档案管理逻辑
  ├── useDownload.ts       # 下载逻辑
  ├── useTauriInvoke.ts    # Tauri 调用封装
  ├── useTauriEvent.ts     # Tauri 事件封装
  └── useAnimations.ts     # 动画配置
```

---

## 7. 性能考虑

### 7.1 性能模式

保留 `performance` 模式:
- 完全禁用动画
- 简化视觉效果
- 适合低端设备

### 7.2 优化策略

- `React.memo` 包裹所有组件
- `useMemo` 缓存过滤/计算结果
- `useCallback` 缓存回调函数
- 列表虚拟化 (如有长列表)
- 图片/资源懒加载

---

## 8. 可访问性

- 所有交互元素可键盘访问
- Focus 可见指示
- ARIA 标签
- 颜色对比度符合 WCAG AA
- Screen reader 支持
- `prefers-reduced-motion` 尊重

---

## 9. 技术栈

| 类别 | 技术 | 备注 |
|------|------|------|
| 框架 | React 19 |  |
| 构建 | Vite |  |
| 语言 | TypeScript |  |
| 样式 | CSS Variables + Framer Motion | 无 Tailwind |
| 动画 | Framer Motion | Cinema Mode |
| 状态 | React Context (拆分后) |  |
| 后端 | Tauri v2 |  |

---

## 10. 实施顺序

1. **Phase 1**: 搭建项目结构 + CSS 设计系统
2. **Phase 2**: 实现侧边栏导航组件
3. **Phase 3**: 实现首页 + 状态卡片
4. **Phase 4**: 实现启动页 + 版本选择器
5. **Phase 5**: 实现设置页
6. **Phase 6**: 实现新闻页
7. **Phase 7**: 状态管理重构 (Context 拆分)
8. **Phase 8**: 动效调试 + 性能优化
9. **Phase 9**: 可访问性 + 测试

---

## 11. 设计预览

详见 `.superpowers/brainstorm/545-1777206315/` 目录下的可视化 mockups。
