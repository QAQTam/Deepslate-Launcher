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