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
