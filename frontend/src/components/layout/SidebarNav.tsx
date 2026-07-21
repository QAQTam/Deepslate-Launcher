import { motion } from 'framer-motion';

interface NavItem {
  id: string;
  label: string;
  /** CSS color for the nav dot (e.g. 'var(--nav-launch)') */
  color?: string;
}

interface SidebarNavProps {
  items: NavItem[];
  activeId: string;
  collapsed?: boolean;
  onNavigate: (id: string) => void;
}

export function SidebarNav({ items, activeId, collapsed, onNavigate }: SidebarNavProps) {
  return (
    <nav className="nav-section">
      {items.map((item) => {
        const isActive = activeId === item.id;
        const dotColor = item.color ?? 'var(--text-muted)';

        return (
          <button
            key={item.id}
            className={`nav-btn${isActive ? ' active' : ''}`}
            onClick={() => onNavigate(item.id)}
            style={{
              borderRadius: 'var(--radius-md)',
            }}
          >
            <motion.span
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: '10px',
                width: '100%',
              }}
              whileHover={{ x: 2 }}
              whileTap={{ scale: 0.98 }}
            >
              {/* Colored dot */}
              <span
                style={{
                  width: '8px',
                  height: '8px',
                  borderRadius: '50%',
                  flexShrink: 0,
                  background: dotColor,
                  boxShadow: isActive
                    ? `0 0 6px ${dotColor}`
                    : 'none',
                  transition: 'box-shadow 0.2s ease',
                }}
              />

              {!collapsed && (
                <span
                  style={{
                    fontSize: '0.85rem',
                    fontWeight: isActive ? 600 : 400,
                    color: isActive ? 'var(--text-primary)' : 'var(--text-secondary)',
                    whiteSpace: 'nowrap',
                  }}
                >
                  {item.label}
                </span>
              )}
            </motion.span>
          </button>
        );
      })}
    </nav>
  );
}
