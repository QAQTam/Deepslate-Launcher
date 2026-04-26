import { useEffect } from 'react'
import { motion } from 'framer-motion'
import { PageContainer } from '../layout/PageContainer'
import { useVersion } from '../../contexts/VersionContext'
import { useProfile } from '../../contexts/ProfileContext'
import { useConfig } from '../../contexts/ConfigContext'
import { useGame } from '../../contexts/GameContext'

export function HomePage() {
  const { selectedVersion, versionType } = useVersion()
  const { selectedProfile } = useProfile()
  const { status } = useConfig()
  const { gameRunning, launching } = useGame()

  const gameStatus = launching ? 'launching' : gameRunning ? 'running' : 'idle'
  const statusText = launching ? '启动中...' : gameRunning ? '游戏中' : '已退出'

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
        <StatusCard title="当前版本" value={selectedVersion || '未选择'} />
        <StatusCard title="当前档案" value={selectedProfile?.name || '未登录'} />
        <StatusCard title="游戏状态" value={statusText} status={gameStatus} />
        <StatusCard title="版本类型" value={versionType === 'release' ? '正式版' : versionType === 'snapshot' ? '测试版' : '历史版'} />
      </motion.div>

      {/* Status Bar */}
      <motion.div
        variants={{ initial: { opacity: 0, y: 20 }, animate: { opacity: 1, y: 0 } }}
        style={{
          marginTop: '24px',
          padding: '16px 20px',
          background: 'rgba(139, 92, 246, 0.1)',
          borderRadius: 'var(--radius-lg)',
          border: '1px solid rgba(139, 92, 246, 0.2)',
        }}
      >
        <p style={{
          fontSize: '0.8rem',
          fontFamily: 'var(--font-mono)',
          color: 'var(--text-secondary)',
        }}>
          {status}
        </p>
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
          <motion.span
            animate={status === 'running' ? { scale: [1, 1.2, 1] } : { opacity: [1, 0.5, 1] }}
            transition={{ duration: status === 'running' ? 1 : 0.8, repeat: Infinity }}
            style={{
              width: '8px',
              height: '8px',
              borderRadius: '50%',
              background: statusColors[status],
            }}
          />
        )}
      </div>
    </motion.div>
  )
}
