import { SidebarNav } from './SidebarNav';
import { motion, AnimatePresence } from 'framer-motion';

interface SidebarProps {
  activePage: string;
  onNavigate: (page: string) => void;
  collapsed: boolean;
  onToggleCollapse: () => void;
}

export function Sidebar({ activePage, onNavigate, collapsed, onToggleCollapse }: SidebarProps) {
  const navItems = [
    { id: 'home',    label: '首页',  color: '#b8a088' },
    { id: 'launch',  label: '启动',  color: 'var(--nav-launch)' },
    { id: 'settings',label: '设置',  color: 'var(--nav-settings)' },
  ];

  return (
    <motion.aside
      className="sidebar"
      initial={false}
      animate={{ width: collapsed ? 56 : 220 }}
      transition={{ duration: 0.2, ease: 'easeInOut' }}
      style={{
        overflow: 'hidden',
        flexShrink: 0,
        display: 'flex',
        flexDirection: 'column',
        padding: 'var(--space-md) 8px',
      }}
    >
      {/* Logo / brand */}
      <AnimatePresence mode="wait">
        {!collapsed && (
          <motion.div
            key="brand"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className="sidebar-header"
            style={{ paddingLeft: '8px' }}
          >
            Obsidian Portal
          </motion.div>
        )}
      </AnimatePresence>

      {/* Navigation */}
      <SidebarNav
        items={navItems}
        activeId={activePage}
        collapsed={collapsed}
        onNavigate={onNavigate}
      />

      {/* Spacer */}
      <div style={{ flex: 1 }} />

      {/* Collapse toggle */}
      <button
        onClick={onToggleCollapse}
        className="btn-ghost"
        style={{
          padding: '8px',
          width: '100%',
          display: 'flex',
          justifyContent: 'center',
          alignItems: 'center',
          fontSize: '0.8rem',
          color: 'var(--text-muted)',
          marginBottom: 'var(--space-sm)',
        }}
        title={collapsed ? '展开侧边栏' : '折叠侧边栏'}
      >
        {collapsed ? '▶' : '◀'}
      </button>

      {/* User section (bottom) */}
      <AnimatePresence mode="wait">
        {!collapsed && (
          <motion.div
            key="user"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className="sidebar-footer"
            style={{ padding: '8px' }}
          >
            <div style={{
              display: 'flex',
              alignItems: 'center',
              gap: '10px',
            }}>
              <div style={{
                width: '28px',
                height: '28px',
                borderRadius: '50%',
                background: '#e0d6c4',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                fontSize: '0.7rem',
                color: 'var(--text-secondary)',
                flexShrink: 0,
              }}>
                S
              </div>
              <div>
                <div style={{
                  fontSize: '0.8rem',
                  fontWeight: 500,
                  color: 'var(--text-primary)',
                }}>
                  Steve
                </div>
                <div style={{
                  fontSize: '0.65rem',
                  color: 'var(--text-muted)',
                }}>
                  microsoft
                </div>
              </div>
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </motion.aside>
  );
}
