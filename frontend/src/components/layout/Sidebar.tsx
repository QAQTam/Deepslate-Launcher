import { motion } from 'framer-motion'
import { SidebarNav } from './SidebarNav'

const NAV_ITEMS = [
  { id: 'home', label: '首页', icon: '🏠' },
  { id: 'launch', label: '启动', icon: '🎮' },
  { id: 'settings', label: '设置', icon: '⚙️' },
]

interface SidebarProps {
  activePage: string
  onNavigate: (page: string) => void
  collapsed: boolean
  onToggleCollapse: () => void
}

export function Sidebar({ activePage, onNavigate, collapsed, onToggleCollapse }: SidebarProps) {
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
          onClick={onToggleCollapse}
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