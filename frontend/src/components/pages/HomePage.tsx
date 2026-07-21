import { motion } from 'framer-motion'
import { useAppState } from '../../hooks/useAppState'
import { ProfileSelector } from '../ui/ProfileSelector'
import { ProfileModal } from '../../modals/ProfileModal'

export function HomePage() {
  const {
    profiles, selectedProfile, setSelectedProfile,
    showProfileModal, setShowProfileModal,
    versions, status,
  } = useAppState()

  return (
    <div style={{ padding: 'var(--space-lg)', maxWidth: '800px', margin: '0 auto' }}>
      {/* Hero / Welcome */}
      <motion.div
        initial={{ opacity: 0, y: 20 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.4 }}
        style={{
          padding: 'var(--space-lg) 0',
          marginBottom: 'var(--space-lg)',
        }}
      >
        <h1 style={{
          fontFamily: 'var(--font-display)',
          fontSize: '2rem',
          fontWeight: 700,
          color: 'var(--text-primary)',
          marginBottom: 'var(--space-sm)',
        }}>
          {selectedProfile
            ? `欢迎回来，${selectedProfile.name}`
            : '欢迎使用 Obsidian Portal'}
        </h1>
        <p style={{
          color: 'var(--text-secondary)',
          fontSize: '1rem',
          maxWidth: '500px',
        }}>
          {selectedProfile
            ? '准备好开始新的冒险了吗？'
            : '请先登录或创建档案以开始使用'}
        </p>

        {/* Login CTA — shown when not logged in */}
        {!selectedProfile && (
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            transition={{ delay: 0.2 }}
            style={{ marginTop: 'var(--space-lg)' }}
          >
            <button
              className="btn-primary"
              onClick={() => setShowProfileModal(true)}
              style={{
                padding: '14px 32px',
                fontSize: '1.05rem',
              }}
            >
              🔑 登录 / 创建档案
            </button>
            <p style={{
              marginTop: 'var(--space-sm)',
              fontSize: '0.8rem',
              color: 'var(--text-muted)',
            }}>
              支持 Microsoft 在线登录或离线账户
            </p>
          </motion.div>
        )}
      </motion.div>

      {/* Status cards */}
      <div style={{
        display: 'grid',
        gridTemplateColumns: 'repeat(auto-fill, minmax(220px, 1fr))',
        gap: 'var(--space-md)',
        marginBottom: 'var(--space-lg)',
      }}>
        {/* Profile card */}
        <div className="card" style={{ position: 'relative' }}>
          <div style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            marginBottom: 'var(--space-sm)',
          }}>
            <span style={{
              fontSize: '0.7rem',
              textTransform: 'uppercase',
              letterSpacing: '0.08em',
              color: 'var(--text-muted)',
              fontWeight: 600,
            }}>
              当前档案
            </span>
            <button
              className="btn-ghost"
              onClick={() => setShowProfileModal(true)}
              style={{
                fontSize: '0.75rem',
                padding: '4px 10px',
              }}
            >
              管理
            </button>
          </div>
          <div style={{
            fontSize: '1rem',
            fontWeight: 600,
            color: selectedProfile ? 'var(--text-primary)' : 'var(--text-muted)',
          }}>
            {selectedProfile?.name || '未登录'}
          </div>
          {selectedProfile && (
            <div style={{
              fontSize: '0.75rem',
              color: 'var(--text-secondary)',
              marginTop: '4px',
            }}>
              {selectedProfile.auth_method === 'Microsoft' ? 'Microsoft 在线账户' : '离线账户'}
            </div>
          )}
        </div>

        {/* Version card */}
        <div className="card">
          <span style={{
            fontSize: '0.7rem',
            textTransform: 'uppercase',
            letterSpacing: '0.08em',
            color: 'var(--text-muted)',
            fontWeight: 600,
            display: 'block',
            marginBottom: 'var(--space-sm)',
          }}>
            已安装版本
          </span>
          <div style={{
            fontSize: '1rem',
            fontWeight: 600,
            color: 'var(--text-primary)',
          }}>
            {versions.filter((v: { type: string }) => v.type === 'release').length} 个
          </div>
          <div style={{
            fontSize: '0.75rem',
            color: 'var(--text-secondary)',
            marginTop: '4px',
          }}>
            {versions.filter((v: { type: string }) => v.type !== 'release').length > 0
              ? `+${versions.filter((v2: { type: string }) => v2.type !== 'release').length} 个快照/旧版`
              : '正式版'}
          </div>
        </div>

        {/* Profile count card */}
        <div className="card">
          <span style={{
            fontSize: '0.7rem',
            textTransform: 'uppercase',
            letterSpacing: '0.08em',
            color: 'var(--text-muted)',
            fontWeight: 600,
            display: 'block',
            marginBottom: 'var(--space-sm)',
          }}>
            档案数量
          </span>
          <div style={{
            fontSize: '1rem',
            fontWeight: 600,
            color: 'var(--text-primary)',
          }}>
            {profiles.length} 个
          </div>
          <div style={{
            fontSize: '0.75rem',
            color: 'var(--text-secondary)',
            marginTop: '4px',
          }}>
            {profiles.filter((p: { auth_method: string }) => p.auth_method === 'Microsoft').length} 在线 &middot;{' '}
            {profiles.filter((p: { auth_method: string }) => p.auth_method === 'Offline').length} 离线
          </div>
        </div>
      </div>

      {/* Quick profile picker */}
      {profiles.length > 0 && (
        <motion.div
          initial={{ opacity: 0, y: 20 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ delay: 0.3 }}
        >
          <ProfileSelector
            profiles={profiles}
            selectedProfile={selectedProfile}
            onSelect={setSelectedProfile}
          />
        </motion.div>
      )}

      {/* Profile modal */}
      {showProfileModal && <ProfileModal />}

      {/* Status bar area */}
      {status && (
        <div style={{
          marginTop: 'var(--space-lg)',
          padding: 'var(--space-md)',
          background: 'var(--bg-input)',
          borderRadius: 'var(--radius-md)',
          fontSize: '0.85rem',
          color: 'var(--text-secondary)',
        }}>
          {status}
        </div>
      )}
    </div>
  )
}
