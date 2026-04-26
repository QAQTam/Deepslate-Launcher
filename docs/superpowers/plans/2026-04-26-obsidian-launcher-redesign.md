# Obsidian Launcher - 破坏性设计实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 对 Obsidian Launcher 前端进行完全重新设计，采用 Void Crystal 美学，打造具有沉浸感的游戏启动器界面。

**Architecture:** 基于 React 19 + TypeScript + Vite，采用 Framer Motion 实现电影级动效，CSS Variables 实现 Void Crystal 配色系统。侧边栏导航 + 内容面板布局，Context 拆分实现状态管理。

**Tech Stack:** React 19, TypeScript, Vite, Framer Motion, CSS Variables, Tauri v2

---

## 文件结构

```
frontend/src/
├── index.css              # Void Crystal CSS 设计系统
├── main.tsx               # 入口
├── App.tsx                # 根组件 + 路由
├── contexts/              # 状态管理 (Phase 7)
│   ├── AppProvider.tsx
│   ├── VersionContext.tsx
│   ├── ProfileContext.tsx
│   ├── DownloadContext.tsx
│   ├── ConfigContext.tsx
│   └── GameContext.tsx
├── hooks/                 # 自定义 hooks
│   ├── useVersion.ts
│   ├── useProfile.ts
│   ├── useDownload.ts
│   └── useAnimations.ts
├── components/
│   ├── layout/
│   │   ├── Sidebar.tsx           # 侧边栏导航
│   │   ├── SidebarNav.tsx        # 导航项
│   │   └── PageContainer.tsx     # 页面容器
│   ├── ui/
│   │   ├── Button.tsx            # 按钮组件
│   │   ├── Card.tsx              # 玻璃卡片
│   │   ├── Input.tsx             # 输入框
│   │   ├── Select.tsx            # 选择器
│   │   ├── Toggle.tsx            # 开关
│   │   ├── Modal.tsx             # 模态框
│   │   └── Progress.tsx          # 进度条
│   └── pages/
│       ├── HomePage.tsx          # 首页
│       ├── LaunchPage.tsx        # 启动页
│       ├── SettingsPage.tsx      # 设置页
│       └── NewsPage.tsx          # 新闻页
├── modals/
│   └── SetupModal.tsx            # 首次设置向导
└── services/
    └── newsApi.ts                 # 新闻 API 服务
```

---

## Phase 1: CSS 设计系统

### Task 1: 创建 Void Crystal CSS 设计系统

**Files:**
- Modify: `frontend/src/index.css`

- [ ] **Step 1: 备份现有 CSS**

备份当前 `index.css` 内容到注释或新文件

- [ ] **Step 2: 创建 Void Crystal 色彩变量**

```css
:root {
  /* 背景层次 */
  --bg-deep: #0f0d1a;
  --bg-primary: #1e1b4b;
  --bg-elevated: #312e81;
  --bg-card: #4c1d95;

  /* 强调色 */
  --accent-primary: #8b5cf6;
  --accent-glow: rgba(139, 92, 246, 0.4);
  --accent-secondary: #6366f1;
  --accent-hot: #a78bfa;

  /* 发光效果 */
  --glow-purple: 0 0 20px rgba(139, 92, 246, 0.5);
  --glow-border: 0 0 0 1px rgba(139, 92, 246, 0.3);

  /* 文字层次 */
  --text-primary: #f5f3ff;
  --text-secondary: #c4b5fd;
  --text-muted: #8b5cf6;

  /* 间距 */
  --space-xs: 4px;
  --space-sm: 8px;
  --space-md: 16px;
  --space-lg: 24px;
  --space-xl: 32px;
  --space-2xl: 48px;

  /* 圆角 */
  --radius-sm: 8px;
  --radius-md: 12px;
  --radius-lg: 16px;
  --radius-xl: 24px;

  /* 字体 */
  --font-display: 'Outfit', sans-serif;
  --font-mono: 'JetBrains Mono', monospace;
  --font-body: system-ui, -apple-system, sans-serif;
}
```

- [ ] **Step 3: 创建基础样式重置**

```css
*, *::before, *::after {
  box-sizing: border-box;
  margin: 0;
  padding: 0;
}

html {
  font-size: 16px;
  scroll-behavior: smooth;
}

body {
  font-family: var(--font-body);
  background: var(--bg-deep);
  color: var(--text-primary);
  min-height: 100vh;
  -webkit-font-smoothing: antialiased;
  overflow-x: hidden;
}

#root {
  min-height: 100vh;
}
```

- [ ] **Step 4: 创建 GlassCard 样式**

```css
.glass-card {
  background: rgba(76, 29, 149, 0.3);
  backdrop-filter: blur(12px);
  border: 1px solid rgba(139, 92, 246, 0.2);
  border-radius: var(--radius-lg);
  box-shadow: var(--glow-border);
  transition: all 0.3s cubic-bezier(0.4, 0, 0.2, 1);
}

.glass-card:hover {
  transform: scale(1.02);
  box-shadow: var(--glow-purple);
  border-color: rgba(139, 92, 246, 0.5);
}
```

- [ ] **Step 5: 创建按钮样式**

```css
.btn {
  padding: 12px 24px;
  border-radius: var(--radius-md);
  font-weight: 600;
  cursor: pointer;
  transition: all 0.25s cubic-bezier(0.4, 0, 0.2, 1);
  border: none;
  font-family: var(--font-body);
}

.btn-primary {
  background: linear-gradient(135deg, var(--accent-primary), var(--accent-secondary));
  color: white;
  box-shadow: var(--glow-purple);
}

.btn-primary:hover {
  box-shadow: 0 0 30px rgba(139, 92, 246, 0.6);
}

.btn-primary:active {
  transform: scale(0.98);
}

.btn-secondary {
  background: transparent;
  color: var(--text-secondary);
  border: 1px solid rgba(139, 92, 246, 0.3);
}

.btn-secondary:hover {
  background: rgba(139, 92, 246, 0.1);
  border-color: var(--accent-primary);
}

.btn-danger {
  background: rgba(239, 68, 68, 0.1);
  color: #ef4444;
  border: 1px solid rgba(239, 68, 68, 0.3);
}
```

- [ ] **Step 6: 创建表单样式**

```css
.input {
  width: 100%;
  padding: 12px 16px;
  background: rgba(30, 27, 75, 0.6);
  border: 1px solid rgba(139, 92, 246, 0.2);
  border-radius: var(--radius-md);
  color: var(--text-primary);
  font-size: 0.9rem;
  transition: all 0.2s ease;
}

.input:focus {
  outline: none;
  border-color: var(--accent-primary);
  box-shadow: 0 0 0 3px rgba(139, 92, 246, 0.2);
}

.input::placeholder {
  color: var(--text-muted);
}

.select {
  appearance: none;
  cursor: pointer;
  background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' viewBox='0 0 24 24' fill='none' stroke='%238b5cf6' stroke-width='2'%3E%3Cpath d='M6 9l6 6 6-6'/%3E%3C/svg%3E");
  background-repeat: no-repeat;
  background-position: right 12px center;
  padding-right: 36px;
}
```

- [ ] **Step 7: 创建进度条样式**

```css
.progress-bar {
  width: 100%;
  height: 8px;
  background: rgba(30, 27, 75, 0.6);
  border-radius: 9999px;
  overflow: hidden;
}

.progress-fill {
  height: 100%;
  background: linear-gradient(90deg, var(--accent-secondary), var(--accent-primary));
  border-radius: 9999px;
  transition: width 0.4s cubic-bezier(0.4, 0, 0.2, 1);
  position: relative;
}

.progress-fill::after {
  content: '';
  position: absolute;
  inset: 0;
  background: linear-gradient(90deg, transparent, rgba(255,255,255,0.3), transparent);
  animation: shimmer 2s infinite;
}

@keyframes shimmer {
  0% { transform: translateX(-100%); }
  100% { transform: translateX(100%); }
}
```

- [ ] **Step 8: 创建 Performance Mode**

```css
[data-ui-mode="performance"] *,
[data-ui-mode="performance"] *::before,
[data-ui-mode="performance"] *::after {
  animation-duration: 0.001ms !important;
  transition-duration: 0.001ms !important;
}

[data-ui-mode="performance"] .glass-card:hover {
  transform: none;
}
```

- [ ] **Step 9: 提交**

```bash
git add frontend/src/index.css
git commit -m "feat(frontend): add Void Crystal CSS design system"
```

---

### Task 2: 安装 Framer Motion

**Files:**
- Modify: `frontend/package.json`

- [ ] **Step 1: 添加 Framer Motion 依赖**

```bash
cd frontend && pnpm add framer-motion
```

- [ ] **Step 2: 提交**

```bash
git add frontend/package.json frontend/pnpm-lock.yaml
git commit -m "feat(frontend): add framer-motion for animations"
```

---

## Phase 2: 侧边栏导航组件

### Task 3: 创建 Sidebar 组件

**Files:**
- Create: `frontend/src/components/layout/Sidebar.tsx`
- Create: `frontend/src/components/layout/SidebarNav.tsx`
- Create: `frontend/src/components/layout/PageContainer.tsx`

- [ ] **Step 1: 创建 SidebarNav 导航项组件**

```tsx
import { motion } from 'framer-motion'
import { useState } from 'react'

interface NavItem {
  id: string
  label: string
  icon: React.ReactNode
}

interface SidebarNavProps {
  items: NavItem[]
  activeId: string
  onSelect: (id: string) => void
  collapsed: boolean
}

export function SidebarNav({ items, activeId, onSelect, collapsed }: SidebarNavProps) {
  return (
    <nav style={{ display: 'flex', flexDirection: 'column', gap: '4px' }}>
      {items.map((item) => (
        <motion.button
          key={item.id}
          onClick={() => onSelect(item.id)}
          whileHover={{ scale: 1.02 }}
          whileTap={{ scale: 0.98 }}
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: '12px',
            padding: collapsed ? '12px' : '12px 16px',
            background: activeId === item.id ? 'rgba(139, 92, 246, 0.2)' : 'transparent',
            border: 'none',
            borderRadius: 'var(--radius-md)',
            color: activeId === item.id ? 'var(--accent-primary)' : 'var(--text-secondary)',
            cursor: 'pointer',
            transition: 'all 0.2s ease',
            justifyContent: collapsed ? 'center' : 'flex-start',
            width: '100%',
          }}
        >
          <span style={{ fontSize: '20px' }}>{item.icon}</span>
          {!collapsed && (
            <span style={{ fontWeight: 500 }}>{item.label}</span>
          )}
          {activeId === item.id && (
            <motion.div
              layoutId="activeIndicator"
              style={{
                position: 'absolute',
                left: 0,
                width: '3px',
                height: '24px',
                background: 'var(--accent-primary)',
                borderRadius: '0 2px 2px 0',
              }}
            />
          )}
        </motion.button>
      ))}
    </nav>
  )
}
```

- [ ] **Step 2: 创建 Sidebar 主组件**

```tsx
import { motion } from 'framer-motion'
import { useState, useEffect } from 'react'
import { SidebarNav } from './SidebarNav'

const NAV_ITEMS = [
  { id: 'home', label: '首页', icon: '🏠' },
  { id: 'launch', label: '启动', icon: '🎮' },
  { id: 'settings', label: '设置', icon: '⚙️' },
]

interface SidebarProps {
  activePage: string
  onNavigate: (page: string) => void
}

export function Sidebar({ activePage, onNavigate }: SidebarProps) {
  const [collapsed, setCollapsed] = useState(false)

  useEffect(() => {
    const saved = localStorage.getItem('sidebar-collapsed')
    if (saved) setCollapsed(JSON.parse(saved))
  }, [])

  const toggleCollapsed = () => {
    const newValue = !collapsed
    setCollapsed(newValue)
    localStorage.setItem('sidebar-collapsed', JSON.stringify(newValue))
  }

  return (
    <motion.aside
      animate={{ width: collapsed ? 64 : 220 }}
      transition={{ type: 'spring', stiffness: 300, damping: 30 }}
      style={{
        height: '100vh',
        background: 'var(--bg-primary)',
        borderRight: '1px solid rgba(139, 92, 246, 0.1)',
        display: 'flex',
        flexDirection: 'column',
        position: 'fixed',
        left: 0,
        top: 0,
        zIndex: 100,
      }}
    >
      {/* Logo */}
      <div style={{
        padding: '20px 16px',
        borderBottom: '1px solid rgba(139, 92, 246, 0.1)',
        display: 'flex',
        alignItems: 'center',
        justifyContent: collapsed ? 'center' : 'space-between',
      }}>
        {!collapsed && (
          <h1 style={{
            fontFamily: 'var(--font-display)',
            fontSize: '1.25rem',
            fontWeight: 700,
            background: 'linear-gradient(135deg, var(--accent-primary), var(--accent-secondary))',
            WebkitBackgroundClip: 'text',
            WebkitTextFillColor: 'transparent',
          }}>
            Obsidian
          </h1>
        )}
        <button
          onClick={toggleCollapsed}
          style={{
            background: 'none',
            border: 'none',
            color: 'var(--text-muted)',
            cursor: 'pointer',
            padding: '4px',
            fontSize: '18px',
          }}
        >
          {collapsed ? '→' : '←'}
        </button>
      </div>

      {/* Nav */}
      <div style={{ flex: 1, padding: '16px 8px' }}>
        <SidebarNav
          items={NAV_ITEMS}
          activeId={activePage}
          onSelect={onNavigate}
          collapsed={collapsed}
        />
      </div>
    </motion.aside>
  )
}
```

- [ ] **Step 3: 创建 PageContainer 组件**

```tsx
import { motion } from 'framer-motion'

interface PageContainerProps {
  children: React.ReactNode
  title: string
}

const pageVariants = {
  initial: { opacity: 0, y: 20 },
  animate: { opacity: 1, y: 0 },
  exit: { opacity: 0, y: -20 },
}

const staggerContainer = {
  animate: {
    transition: {
      staggerChildren: 0.1,
    },
  },
}

const staggerItem = {
  initial: { opacity: 0, y: 20 },
  animate: { opacity: 1, y: 0 },
}

export function PageContainer({ children, title }: PageContainerProps) {
  return (
    <motion.div
      initial="initial"
      animate="animate"
      exit="exit"
      variants={pageVariants}
      transition={{ duration: 0.3 }}
      style={{ padding: '32px' }}
    >
      <motion.h2
        variants={staggerItem}
        style={{
          fontFamily: 'var(--font-display)',
          fontSize: '1.5rem',
          fontWeight: 700,
          marginBottom: '24px',
          color: 'var(--text-primary)',
        }}
      >
        {title}
      </motion.h2>
      <motion.div variants={staggerContainer}>
        {children}
      </motion.div>
    </motion.div>
  )
}
```

- [ ] **Step 4: 提交**

```bash
git add frontend/src/components/layout/Sidebar.tsx frontend/src/components/layout/SidebarNav.tsx frontend/src/components/layout/PageContainer.tsx
git commit -m "feat(frontend): add sidebar navigation components"
```

---

## Phase 3: 首页实现

### Task 4: 创建 HomePage 组件

**Files:**
- Create: `frontend/src/components/pages/HomePage.tsx`

- [ ] **Step 1: 创建 HomePage 组件**

```tsx
import { motion } from 'framer-motion'
import { PageContainer } from '../layout/PageContainer'

export function HomePage() {
  return (
    <PageContainer title="首页">
      <motion.div
        variants={{
          animate: { transition: { staggerChildren: 0.1 } },
        }}
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fit, minmax(280px, 1fr))',
          gap: '20px',
        }}
      >
        <StatusCard title="当前版本" value="1.20.4" />
        <StatusCard title="当前档案" value="Player123" />
        <StatusCard title="游戏状态" value="已退出" status="idle" />
      </motion.div>
    </PageContainer>
  )
}

interface StatusCardProps {
  title: string
  value: string
  status?: 'idle' | 'running' | 'launching'
}

function StatusCard({ title, value, status = 'idle' }: StatusCardProps) {
  const statusColors = {
    idle: 'var(--text-muted)',
    running: '#22c55e',
    launching: 'var(--accent-primary)',
  }

  return (
    <motion.div
      variants={{
        initial: { opacity: 0, y: 20 },
        animate: { opacity: 1, y: 0 },
      }}
      whileHover={{ scale: 1.02, boxShadow: 'var(--glow-purple)' }}
      className="glass-card"
      style={{ padding: '20px' }}
    >
      <p style={{
        fontSize: '0.75rem',
        color: 'var(--text-muted)',
        textTransform: 'uppercase',
        letterSpacing: '0.1em',
        marginBottom: '8px',
      }}>
        {title}
      </p>
      <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
        <span style={{ fontSize: '1.25rem', fontWeight: 600, color: 'var(--text-primary)' }}>
          {value}
        </span>
        {status !== 'idle' && (
          <span style={{
            width: '8px',
            height: '8px',
            borderRadius: '50%',
            background: statusColors[status],
          }} />
        )}
      </div>
    </motion.div>
  )
}
```

- [ ] **Step 2: 提交**

```bash
git add frontend/src/components/pages/HomePage.tsx
git commit -m "feat(frontend): add HomePage component"
```

---

## Phase 4: 启动页实现

### Task 5: 创建 LaunchPage 组件

**Files:**
- Create: `frontend/src/components/pages/LaunchPage.tsx`
- Create: `frontend/src/components/ui/VersionSelector.tsx`
- Create: `frontend/src/components/ui/ProfileSelector.tsx`
- Create: `frontend/src/components/ui/LaunchButton.tsx`

- [ ] **Step 1: 创建 VersionSelector 组件**

```tsx
import { motion } from 'framer-motion'

const TABS = [
  { id: 'release', label: '正式版' },
  { id: 'snapshot', label: '测试版' },
  { id: 'old', label: '历史版' },
] as const

interface VersionSelectorProps {
  activeTab: typeof TABS[number]['id']
  onTabChange: (tab: typeof TABS[number]['id']) => void
  versions: string[]
  selectedVersion: string
  onVersionChange: (version: string) => void
}

export function VersionSelector({
  activeTab,
  onTabChange,
  versions,
  selectedVersion,
  onVersionChange,
}: VersionSelectorProps) {
  return (
    <div className="glass-card" style={{ padding: '20px' }}>
      <h3 style={{
        fontFamily: 'var(--font-display)',
        fontSize: '0.8rem',
        color: 'var(--text-muted)',
        textTransform: 'uppercase',
        letterSpacing: '0.1em',
        marginBottom: '16px',
      }}>
        选择版本
      </h3>

      {/* Tabs */}
      <div style={{
        display: 'flex',
        gap: '4px',
        background: 'rgba(30, 27, 75, 0.4)',
        padding: '4px',
        borderRadius: 'var(--radius-md)',
        marginBottom: '16px',
      }}>
        {TABS.map((tab) => (
          <button
            key={tab.id}
            onClick={() => onTabChange(tab.id)}
            style={{
              flex: 1,
              padding: '10px 12px',
              border: 'none',
              borderRadius: 'var(--radius-sm)',
              background: activeTab === tab.id ? 'var(--bg-card)' : 'transparent',
              color: activeTab === tab.id ? 'var(--accent-primary)' : 'var(--text-muted)',
              fontWeight: 600,
              cursor: 'pointer',
              transition: 'all 0.2s ease',
            }}
          >
            {tab.label}
          </button>
        ))}
      </div>

      {/* Version Select */}
      <select
        value={selectedVersion}
        onChange={(e) => onVersionChange(e.target.value)}
        className="input select"
      >
        {versions.map((v) => (
          <option key={v} value={v}>{v}</option>
        ))}
      </select>
    </div>
  )
}
```

- [ ] **Step 2: 创建 ProfileSelector 组件**

```tsx
import { motion } from 'framer-motion'

interface Profile {
  name: string
  uuid: string
  auth_method: 'Offline' | 'Microsoft'
}

interface ProfileSelectorProps {
  profiles: Profile[]
  selectedProfile: Profile | null
  onSelect: (profile: Profile) => void
}

export function ProfileSelector({
  profiles,
  selectedProfile,
  onSelect,
}: ProfileSelectorProps) {
  return (
    <div className="glass-card" style={{ padding: '20px' }}>
      <h3 style={{
        fontFamily: 'var(--font-display)',
        fontSize: '0.8rem',
        color: 'var(--text-muted)',
        textTransform: 'uppercase',
        letterSpacing: '0.1em',
        marginBottom: '16px',
      }}>
        选择档案
      </h3>

      <div style={{ display: 'flex', flexDirection: 'column', gap: '8px' }}>
        {profiles.map((profile) => (
          <motion.button
            key={profile.name}
            onClick={() => onSelect(profile)}
            whileHover={{ scale: 1.01 }}
            whileTap={{ scale: 0.99 }}
            style={{
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'space-between',
              padding: '14px 16px',
              background: selectedProfile?.name === profile.name
                ? 'rgba(139, 92, 246, 0.15)'
                : 'rgba(30, 27, 75, 0.4)',
              border: `1px solid ${
                selectedProfile?.name === profile.name
                  ? 'var(--accent-primary)'
                  : 'rgba(139, 92, 246, 0.1)'
              }`,
              borderRadius: 'var(--radius-md)',
              cursor: 'pointer',
              transition: 'all 0.2s ease',
            }}
          >
            <div style={{ textAlign: 'left' }}>
              <p style={{ color: 'var(--text-primary)', fontWeight: 500 }}>
                {profile.name}
              </p>
              <p style={{ color: 'var(--text-muted)', fontSize: '0.75rem', fontFamily: 'var(--font-mono)' }}>
                {profile.uuid}
              </p>
            </div>
            <span style={{
              fontSize: '0.65rem',
              padding: '4px 8px',
              borderRadius: '9999px',
              background: profile.auth_method === 'Microsoft'
                ? 'rgba(139, 92, 246, 0.2)'
                : 'rgba(100, 100, 100, 0.2)',
              color: profile.auth_method === 'Microsoft'
                ? 'var(--accent-primary)'
                : 'var(--text-muted)',
            }}>
              {profile.auth_method}
            </span>
          </motion.button>
        ))}
      </div>
    </div>
  )
}
```

- [ ] **Step 3: 创建 LaunchButton 组件**

```tsx
import { motion } from 'framer-motion'

interface LaunchButtonProps {
  onClick: () => void
  disabled: boolean
  loading: boolean
}

export function LaunchButton({ onClick, disabled, loading }: LaunchButtonProps) {
  return (
    <motion.button
      onClick={onClick}
      disabled={disabled || loading}
      whileHover={{ scale: disabled ? 1 : 1.02 }}
      whileTap={{ scale: disabled ? 1 : 0.98 }}
      style={{
        width: '100%',
        padding: '20px 32px',
        fontSize: '1.125rem',
        fontWeight: 700,
        background: disabled
          ? 'rgba(100, 100, 100, 0.2)'
          : 'linear-gradient(135deg, var(--accent-primary), var(--accent-secondary))',
        border: 'none',
        borderRadius: 'var(--radius-lg)',
        color: disabled ? 'var(--text-muted)' : 'white',
        cursor: disabled ? 'not-allowed' : 'pointer',
        boxShadow: disabled ? 'none' : 'var(--glow-purple)',
        transition: 'all 0.3s ease',
      }}
    >
      {loading ? '启动中...' : '启动游戏'}
    </motion.button>
  )
}
```

- [ ] **Step 4: 创建 LaunchPage 组件**

```tsx
import { useState } from 'react'
import { motion } from 'framer-motion'
import { PageContainer } from '../layout/PageContainer'
import { VersionSelector } from '../ui/VersionSelector'
import { ProfileSelector } from '../ui/ProfileSelector'
import { LaunchButton } from '../ui/LaunchButton'

export function LaunchPage() {
  const [activeTab, setActiveTab] = useState<'release' | 'snapshot' | 'old'>('release')
  const [selectedVersion, setSelectedVersion] = useState('')

  return (
    <PageContainer title="启动游戏">
      <div style={{
        display: 'grid',
        gridTemplateColumns: 'repeat(auto-fit, minmax(300px, 1fr))',
        gap: '20px',
      }}>
        <motion.div
          variants={{
            animate: { transition: { staggerChildren: 0.1 } },
          }}
        >
          <motion.div variants={{ initial: { opacity: 0, y: 20 }, animate: { opacity: 1, y: 0 } }}>
            <VersionSelector
              activeTab={activeTab}
              onTabChange={setActiveTab}
              versions={['1.20.4', '1.20.2', '1.19.4']}
              selectedVersion={selectedVersion}
              onVersionChange={setSelectedVersion}
            />
          </motion.div>

          <motion.div variants={{ initial: { opacity: 0, y: 20 }, animate: { opacity: 1, y: 0 } }} style={{ marginTop: '20px' }}>
            <ProfileSelector
              profiles={[
                { name: 'Player123', uuid: 'abc-123', auth_method: 'Offline' },
                { name: 'Steve', uuid: 'def-456', auth_method: 'Microsoft' },
              ]}
              selectedProfile={null}
              onSelect={() => {}}
            />
          </motion.div>
        </motion.div>

        <motion.div variants={{ initial: { opacity: 0, y: 20 }, animate: { opacity: 1, y: 0 } }}>
          <div className="glass-card" style={{ padding: '20px', height: '100%' }}>
            <h3 style={{
              fontFamily: 'var(--font-display)',
              fontSize: '0.8rem',
              color: 'var(--text-muted)',
              textTransform: 'uppercase',
              letterSpacing: '0.1em',
              marginBottom: '16px',
            }}>
              游戏设置
            </h3>
            <div style={{ display: 'flex', flexDirection: 'column', gap: '12px' }}>
              <div>
                <label style={{ fontSize: '0.8rem', color: 'var(--text-muted)' }}>Java 运行时</label>
                <select className="input select" style={{ marginTop: '4px' }}>
                  <option>Java 17</option>
                  <option>Java 21</option>
                </select>
              </div>
              <div>
                <label style={{ fontSize: '0.8rem', color: 'var(--text-muted)' }}>并发下载数</label>
                <select className="input select" style={{ marginTop: '4px' }}>
                  <option>32</option>
                  <option>16</option>
                  <option>8</option>
                </select>
              </div>
            </div>
          </div>
        </motion.div>
      </div>

      <motion.div
        variants={{ initial: { opacity: 0, y: 20 }, animate: { opacity: 1, y: 0 } }}
        style={{ marginTop: '24px' }}
      >
        <LaunchButton onClick={() => {}} disabled={!selectedVersion} loading={false} />
      </motion.div>
    </PageContainer>
  )
}
```

- [ ] **Step 5: 提交**

```bash
git add frontend/src/components/pages/LaunchPage.tsx frontend/src/components/ui/VersionSelector.tsx frontend/src/components/ui/ProfileSelector.tsx frontend/src/components/ui/LaunchButton.tsx
git commit -m "feat(frontend): add LaunchPage with version/profile selectors"
```

---

## Phase 5: 设置页实现

### Task 6: 创建 SettingsPage 组件

**Files:**
- Create: `frontend/src/components/pages/SettingsPage.tsx`

- [ ] **Step 1: 创建 SettingsPage 组件**

```tsx
import { motion } from 'framer-motion'
import { PageContainer } from '../layout/PageContainer'

export function SettingsPage() {
  return (
    <PageContainer title="设置">
      <div style={{ display: 'flex', flexDirection: 'column', gap: '20px', maxWidth: '600px' }}>
        <motion.div variants={{ initial: { opacity: 0, y: 20 }, animate: { opacity: 1, y: 0 } }}>
          <div className="glass-card" style={{ padding: '20px' }}>
            <h3 style={{
              fontFamily: 'var(--font-display)',
              fontSize: '0.8rem',
              color: 'var(--text-muted)',
              textTransform: 'uppercase',
              letterSpacing: '0.1em',
              marginBottom: '16px',
            }}>
              目录配置
            </h3>
            <div style={{ display: 'flex', flexDirection: 'column', gap: '12px' }}>
              <div>
                <label style={{ fontSize: '0.8rem', color: 'var(--text-muted)', display: 'block', marginBottom: '4px' }}>
                  根目录
                </label>
                <div style={{ display: 'flex', gap: '8px' }}>
                  <input
                    type="text"
                    className="input"
                    placeholder="/path/to/games"
                    style={{ flex: 1 }}
                    readOnly
                  />
                  <button className="btn btn-secondary">浏览</button>
                </div>
              </div>
            </div>
          </div>
        </motion.div>

        <motion.div variants={{ initial: { opacity: 0, y: 20 }, animate: { opacity: 1, y: 0 } }}>
          <div className="glass-card" style={{ padding: '20px' }}>
            <h3 style={{
              fontFamily: 'var(--font-display)',
              fontSize: '0.8rem',
              color: 'var(--text-muted)',
              textTransform: 'uppercase',
              letterSpacing: '0.1em',
              marginBottom: '16px',
            }}>
              隔离设置
            </h3>
            <div style={{ display: 'flex', flexDirection: 'column', gap: '12px' }}>
              <label style={{ display: 'flex', alignItems: 'center', gap: '12px', cursor: 'pointer' }}>
                <input type="checkbox" style={{ width: '18px', height: '18px', accentColor: 'var(--accent-primary)' }} />
                <div>
                  <p style={{ color: 'var(--text-primary)', fontWeight: 500 }}>版本隔离</p>
                  <p style={{ color: 'var(--text-muted)', fontSize: '0.8rem' }}>每个版本独立目录</p>
                </div>
              </label>
              <label style={{ display: 'flex', alignItems: 'center', gap: '12px', cursor: 'pointer' }}>
                <input type="checkbox" style={{ width: '18px', height: '18px', accentColor: 'var(--accent-primary)' }} />
                <div>
                  <p style={{ color: 'var(--text-primary)', fontWeight: 500 }}>存档隔离</p>
                  <p style={{ color: 'var(--text-muted)', fontSize: '0.8rem' }}>每版本独立存档</p>
                </div>
              </label>
            </div>
          </div>
        </motion.div>

        <motion.div variants={{ initial: { opacity: 0, y: 20 }, animate: { opacity: 1, y: 0 } }}>
          <div className="glass-card" style={{ padding: '20px' }}>
            <h3 style={{
              fontFamily: 'var(--font-display)',
              fontSize: '0.8rem',
              color: 'var(--text-muted)',
              textTransform: 'uppercase',
              letterSpacing: '0.1em',
              marginBottom: '16px',
            }}>
              界面模式
            </h3>
            <div style={{
              display: 'flex',
              gap: '8px',
              background: 'rgba(30, 27, 75, 0.4)',
              padding: '4px',
              borderRadius: 'var(--radius-md)',
            }}>
              <button
                className="btn"
                style={{
                  flex: 1,
                  background: 'var(--bg-card)',
                  color: 'var(--accent-primary)',
                }}
              >
                华丽模式
              </button>
              <button
                className="btn"
                style={{
                  flex: 1,
                  background: 'transparent',
                  color: 'var(--text-muted)',
                }}
              >
                性能模式
              </button>
            </div>
          </div>
        </motion.div>

        <motion.div variants={{ initial: { opacity: 0, y: 20 }, animate: { opacity: 1, y: 0 } }}>
          <div className="glass-card" style={{
            padding: '20px',
            borderColor: 'rgba(239, 68, 68, 0.3)',
          }}>
            <h3 style={{
              fontFamily: 'var(--font-display)',
              fontSize: '0.8rem',
              color: '#ef4444',
              textTransform: 'uppercase',
              letterSpacing: '0.1em',
              marginBottom: '16px',
            }}>
              危险区域
            </h3>
            <div style={{ display: 'flex', flexDirection: 'column', gap: '12px' }}>
              <button className="btn btn-danger">清空游戏文件</button>
              <button className="btn btn-danger">重置所有配置</button>
            </div>
          </div>
        </motion.div>
      </div>
    </PageContainer>
  )
}
```

- [ ] **Step 2: 提交**

```bash
git add frontend/src/components/pages/SettingsPage.tsx
git commit -m "feat(frontend): add SettingsPage component"
```

---

## Phase 6: 新闻页实现

### Task 7: 创建 NewsPage 组件和新闻 API

**Files:**
- Create: `frontend/src/services/newsApi.ts`
- Create: `frontend/src/components/pages/NewsPage.tsx`

- [ ] **Step 1: 创建新闻 API 服务**

```typescript
interface NewsItem {
  id: string
  title: string
  description: string
  link: string
  pubDate: string
  category: 'official' | 'community' | 'mod'
}

export async function fetchMinecraftNews(): Promise<NewsItem[]> {
  // 使用 minecraft.net RSS feed
  const RSS_URL = 'https://www.minecraft.net/content/minecraft-net/master/en-us/xml/public.blogs.en-us'

  try {
    const response = await fetch(RSS_URL)
    const text = await response.text()
    // 简单的 XML 解析
    const items: NewsItem[] = []

    // 实际项目中应使用 XML 解析库
    // 此处为简化示例
    return items
  } catch (error) {
    console.error('Failed to fetch news:', error)
    return []
  }
}
```

- [ ] **Step 2: 创建 NewsPage 组件**

```tsx
import { useState, useEffect } from 'react'
import { motion } from 'framer-motion'
import { PageContainer } from '../layout/PageContainer'
import { fetchMinecraftNews, type NewsItem } from '../../services/newsApi'

const CATEGORIES = [
  { id: 'all', label: '全部' },
  { id: 'official', label: '官方' },
  { id: 'community', label: '社区' },
  { id: 'mod', label: 'Mod' },
] as const

export function NewsPage() {
  const [activeCategory, setActiveCategory] = useState<typeof CATEGORIES[number]['id']>('all')
  const [news, setNews] = useState<NewsItem[]>([])
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    async function loadNews() {
      const items = await fetchMinecraftNews()
      setNews(items)
      setLoading(false)
    }
    loadNews()
  }, [])

  const filteredNews = activeCategory === 'all'
    ? news
    : news.filter((item) => item.category === activeCategory)

  return (
    <PageContainer title="新闻动态">
      {/* Category Filter */}
      <motion.div
        variants={{ initial: { opacity: 0, y: 20 }, animate: { opacity: 1, y: 0 } }}
        style={{
          display: 'flex',
          gap: '8px',
          marginBottom: '24px',
          flexWrap: 'wrap',
        }}
      >
        {CATEGORIES.map((cat) => (
          <button
            key={cat.id}
            onClick={() => setActiveCategory(cat.id)}
            style={{
              padding: '8px 16px',
              borderRadius: 'var(--radius-full)',
              border: 'none',
              background: activeCategory === cat.id
                ? 'var(--accent-primary)'
                : 'rgba(139, 92, 246, 0.1)',
              color: activeCategory === cat.id
                ? 'white'
                : 'var(--text-secondary)',
              fontWeight: 500,
              cursor: 'pointer',
              transition: 'all 0.2s ease',
            }}
          >
            {cat.label}
          </button>
        ))}
      </motion.div>

      {/* News Grid */}
      {loading ? (
        <div style={{ textAlign: 'center', padding: '48px', color: 'var(--text-muted)' }}>
          加载中...
        </div>
      ) : (
        <motion.div
          variants={{ animate: { transition: { staggerChildren: 0.1 } } }}
          style={{
            display: 'grid',
            gridTemplateColumns: 'repeat(auto-fill, minmax(300px, 1fr))',
            gap: '20px',
          }}
        >
          {filteredNews.map((item) => (
            <NewsCard key={item.id} item={item} />
          ))}
        </motion.div>
      )}
    </PageContainer>
  )
}

function NewsCard({ item }: { item: NewsItem }) {
  return (
    <motion.a
      href={item.link}
      target="_blank"
      rel="noopener noreferrer"
      variants={{
        initial: { opacity: 0, y: 20 },
        animate: { opacity: 1, y: 0 },
      }}
      whileHover={{ scale: 1.02, boxShadow: 'var(--glow-purple)' }}
      className="glass-card"
      style={{
        padding: '20px',
        display: 'block',
        textDecoration: 'none',
        color: 'inherit',
      }}
    >
      <span style={{
        fontSize: '0.7rem',
        color: 'var(--accent-primary)',
        textTransform: 'uppercase',
        letterSpacing: '0.05em',
      }}>
        {item.category}
      </span>
      <h4 style={{
        fontFamily: 'var(--font-display)',
        fontSize: '1rem',
        fontWeight: 600,
        color: 'var(--text-primary)',
        marginTop: '8px',
        marginBottom: '8px',
      }}>
        {item.title}
      </h4>
      <p style={{
        fontSize: '0.875rem',
        color: 'var(--text-secondary)',
        lineHeight: 1.5,
        display: '-webkit-box',
        WebkitLineClamp: 3,
        WebkitBoxOrient: 'vertical',
        overflow: 'hidden',
      }}>
        {item.description}
      </p>
      <p style={{
        fontSize: '0.75rem',
        color: 'var(--text-muted)',
        marginTop: '12px',
      }}>
        {new Date(item.pubDate).toLocaleDateString('zh-CN')}
      </p>
    </motion.a>
  )
}
```

- [ ] **Step 3: 提交**

```bash
git add frontend/src/services/newsApi.ts frontend/src/components/pages/NewsPage.tsx
git commit -m "feat(frontend): add NewsPage with news feed"
```

---

## Phase 7: 状态管理重构

### Task 8: 拆分 Context

**Files:**
- Create: `frontend/src/contexts/VersionContext.tsx`
- Create: `frontend/src/contexts/ProfileContext.tsx`
- Create: `frontend/src/contexts/DownloadContext.tsx`
- Create: `frontend/src/contexts/ConfigContext.tsx`
- Create: `frontend/src/contexts/GameContext.tsx`
- Create: `frontend/src/contexts/AppProvider.tsx`
- Modify: `frontend/src/App.tsx`

- [ ] **Step 1: 创建 VersionContext**

```tsx
import { createContext, useContext, useState, useCallback, type ReactNode } from 'react'
import { tauriInvoke } from '../../hooks/useTauriInvoke'

interface VersionInfo {
  id: string
  type: 'release' | 'snapshot' | 'old_alpha' | 'old_beta'
  url: string
  releaseTime: string
}

interface VersionContextType {
  versions: VersionInfo[]
  selectedVersion: string
  versionType: 'release' | 'snapshot' | 'old'
  loading: boolean
  loadVersions: () => Promise<void>
  setSelectedVersion: (version: string) => void
  setVersionType: (type: 'release' | 'snapshot' | 'old') => void
  getFilteredVersions: () => VersionInfo[]
}

const VersionContext = createContext<VersionContextType | null>(null)

export function VersionProvider({ children }: { children: ReactNode }) {
  const [versions, setVersions] = useState<VersionInfo[]>([])
  const [selectedVersion, setSelectedVersion] = useState('')
  const [versionType, setVersionType] = useState<'release' | 'snapshot' | 'old'>('release')
  const [loading, setLoading] = useState(false)

  const loadVersions = useCallback(async () => {
    setLoading(true)
    try {
      const vers = await tauriInvoke<VersionInfo[]>('get_versions')
      setVersions(vers)
      const latest = vers.find((v: VersionInfo) => v.type === 'release')
      if (latest) setSelectedVersion(latest.id)
    } catch (e) {
      console.error('Failed to load versions:', e)
    } finally {
      setLoading(false)
    }
  }, [])

  const getFilteredVersions = useCallback(() => {
    return versions.filter((v) => {
      if (versionType === 'release') return v.type === 'release'
      if (versionType === 'snapshot') return v.type === 'snapshot'
      if (versionType === 'old') return v.type === 'old_alpha' || v.type === 'old_beta'
      return true
    })
  }, [versions, versionType])

  return (
    <VersionContext.Provider
      value={{
        versions,
        selectedVersion,
        versionType,
        loading,
        loadVersions,
        setSelectedVersion,
        setVersionType,
        getFilteredVersions,
      }}
    >
      {children}
    </VersionContext.Provider>
  )
}

export function useVersion() {
  const ctx = useContext(VersionContext)
  if (!ctx) throw new Error('useVersion must be used within VersionProvider')
  return ctx
}
```

- [ ] **Step 2: 创建 ProfileContext**

```tsx
import { createContext, useContext, useState, useCallback, type ReactNode } from 'react'
import { tauriInvoke } from '../../hooks/useTauriInvoke'

interface UserProfile {
  name: string
  uuid: string
  auth_method: 'Offline' | 'Microsoft'
  access_token?: string
}

interface ProfileContextType {
  profiles: UserProfile[]
  selectedProfile: UserProfile | null
  loading: boolean
  loadProfiles: () => Promise<void>
  selectProfile: (profile: UserProfile) => void
  createProfile: (name: string) => Promise<void>
  deleteProfile: (name: string) => Promise<void>
}

const ProfileContext = createContext<ProfileContextType | null>(null)

export function ProfileProvider({ children }: { children: ReactNode }) {
  const [profiles, setProfiles] = useState<UserProfile[]>([])
  const [selectedProfile, setSelectedProfile] = useState<UserProfile | null>(null)
  const [loading, setLoading] = useState(false)

  const loadProfiles = useCallback(async () => {
    setLoading(true)
    try {
      const store = await tauriInvoke<{ profiles: UserProfile[]; last_used: string | null }>('get_profiles')
      setProfiles(store.profiles)
      if (store.last_used) {
        const last = store.profiles.find((p: UserProfile) => p.name === store.last_used)
        if (last) setSelectedProfile(last)
      }
    } catch (e) {
      console.error('Failed to load profiles:', e)
    } finally {
      setLoading(false)
    }
  }, [])

  const selectProfile = useCallback((profile: UserProfile) => {
    setSelectedProfile(profile)
  }, [])

  const createProfile = useCallback(async (name: string) => {
    const profile = await tauriInvoke<UserProfile>('create_profile', { name })
    setProfiles((prev) => [...prev, profile])
    setSelectedProfile(profile)
  }, [])

  const deleteProfile = useCallback(async (name: string) => {
    await tauriInvoke('delete_profile', { name })
    setProfiles((prev) => prev.filter((p) => p.name !== name))
    setSelectedProfile((prev) => (prev?.name === name ? null : prev))
  }, [])

  return (
    <ProfileContext.Provider
      value={{
        profiles,
        selectedProfile,
        loading,
        loadProfiles,
        selectProfile,
        createProfile,
        deleteProfile,
      }}
    >
      {children}
    </ProfileContext.Provider>
  )
}

export function useProfile() {
  const ctx = useContext(ProfileContext)
  if (!ctx) throw new Error('useProfile must be used within ProfileProvider')
  return ctx
}
```

- [ ] **Step 3: 创建 AppProvider (根 provider)**

```tsx
import { type ReactNode } from 'react'
import { VersionProvider } from './VersionContext'
import { ProfileProvider } from './ProfileContext'

export function AppProvider({ children }: { children: ReactNode }) {
  return (
    <VersionProvider>
      <ProfileProvider>
        {children}
      </ProfileProvider>
    </VersionProvider>
  )
}
```

- [ ] **Step 4: 更新 App.tsx**

```tsx
import { useState } from 'react'
import { Sidebar } from './components/layout/Sidebar'
import { HomePage } from './components/pages/HomePage'
import { LaunchPage } from './components/pages/LaunchPage'
import { SettingsPage } from './components/pages/SettingsPage'
import { NewsPage } from './components/pages/NewsPage'
import { AppProvider } from './contexts/AppProvider'

function AppContent() {
  const [activePage, setActivePage] = useState('home')

  const renderPage = () => {
    switch (activePage) {
      case 'home':
        return <HomePage />
      case 'launch':
        return <LaunchPage />
      case 'settings':
        return <SettingsPage />
      case 'news':
        return <NewsPage />
      default:
        return <HomePage />
    }
  }

  return (
    <div style={{
      display: 'flex',
      minHeight: '100vh',
      background: 'var(--bg-deep)',
    }}>
      <Sidebar activePage={activePage} onNavigate={setActivePage} />
      <main style={{
        marginLeft: '64px',
        flex: 1,
        minHeight: '100vh',
      }}>
        {renderPage()}
      </main>
    </div>
  )
}

export default function App() {
  return (
    <AppProvider>
      <AppContent />
    </AppProvider>
  )
}
```

- [ ] **Step 5: 提交**

```bash
git add frontend/src/contexts/
git commit -m "refactor(frontend): split AppState into multiple contexts"
```

---

## Phase 8: App.tsx 最终整合

### Task 9: 整合所有组件到 App.tsx

**Files:**
- Modify: `frontend/src/App.tsx`

- [ ] **Step 1: 确认 App.tsx 结构完整**

确保 App.tsx 包含:
- AppProvider 包裹
- Sidebar 导航
- 页面路由逻辑
- 所有页面组件导入

- [ ] **Step 2: 提交**

```bash
git add frontend/src/App.tsx
git commit -m "feat(frontend): integrate all pages with sidebar navigation"
```

---

## Phase 9: 可访问性与性能优化

### Task 10: 添加可访问性支持

**Files:**
- Modify: `frontend/src/components/layout/Sidebar.tsx`
- Modify: `frontend/src/components/ui/LaunchButton.tsx`

- [ ] **Step 1: 为 Sidebar 添加键盘导航**

```tsx
// 在 button 元素上添加
onKeyDown={(e) => {
  if (e.key === 'Enter' || e.key === ' ') {
    onSelect(item.id)
  }
}}

// 添加 aria-label
aria-label={collapsed ? item.label : undefined}
```

- [ ] **Step 2: 为 LaunchButton 添加 ARIA**

```tsx
aria-label="启动游戏"
aria-disabled={disabled}
role="button"
```

- [ ] **Step 3: 提交**

```bash
git add frontend/src/components/
git commit -m "feat(frontend): add accessibility support"
```

---

## 实施总结

| Phase | Task | 描述 |
|-------|------|------|
| 1 | 1-2 | CSS 设计系统 + Framer Motion |
| 2 | 3 | 侧边栏导航组件 |
| 3 | 4 | 首页 |
| 4 | 5 | 启动页 + 版本/档案选择器 |
| 5 | 6 | 设置页 |
| 6 | 7 | 新闻页 |
| 7 | 8 | Context 拆分重构 |
| 8 | 9 | App.tsx 整合 |
| 9 | 10 | 可访问性 + 性能优化 |

---

**Plan complete and saved to `docs/superpowers/plans/2026-04-26-obsidian-launcher-redesign.md`.**

**Two execution options:**

**1. Subagent-Driven (recommended)** - I dispatch a fresh subagent per task, review between tasks, fast iteration

**2. Inline Execution** - Execute tasks in this session using executing-plans, batch execution with checkpoints

Which approach?
