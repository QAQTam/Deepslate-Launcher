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
      aria-label="启动游戏"
      aria-disabled={disabled}
      role="button"
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
