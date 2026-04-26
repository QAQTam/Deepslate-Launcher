import { motion } from 'framer-motion'

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
          onKeyDown={(e) => {
            if (e.key === 'Enter' || e.key === ' ') {
              onSelect(item.id)
            }
          }}
          aria-label={collapsed ? item.label : undefined}
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
            position: 'relative',
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