import { useAppState } from '../hooks/useAppState'

export function ProfileSection() {
  const {
    profiles, selectedProfile, setSelectedProfile,
    handleDeleteProfile, handleResetUuid,
    setShowProfileModal
  } = useAppState()

  return (
    <section className="section">
      <div className="section-header">
        <h2>用户档案</h2>
        <button onClick={() => setShowProfileModal(true)} className="btn-sm btn-secondary">
          新建档案
        </button>
      </div>
      {profiles.length === 0 ? (
        <div className="empty-state">
          <div className="empty-state-icon">👤</div>
          <p>暂无档案，请创建新档案</p>
        </div>
      ) : (
        <div className="profile-list">
          {profiles.map(profile => (
            <div
              key={profile.name}
              className={`profile-item ${selectedProfile?.name === profile.name ? 'selected' : ''}`}
              onClick={() => setSelectedProfile(profile)}
            >
              <div className="profile-info">
                <span className="profile-name">
                  {profile.name}
                  {profile.auth_method === 'Microsoft' && (
                    <span className="auth-badge online">在线</span>
                  )}
                  {profile.auth_method === 'Offline' && (
                    <span className="auth-badge offline">离线</span>
                  )}
                </span>
                <span className="profile-uuid">{profile.uuid}</span>
              </div>
              <button
                className="btn-sm btn-danger"
                onClick={(e) => { e.stopPropagation(); handleDeleteProfile(profile.name) }}
              >
                删除
              </button>
            </div>
          ))}
        </div>
      )}
      {selectedProfile && (
        <div className="profile-actions">
          <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
            <button
              onClick={handleResetUuid}
              className="btn-sm btn-ghost"
              disabled={selectedProfile.auth_method === 'Microsoft'}
              title={selectedProfile.auth_method === 'Microsoft' ? '在线登录的 UUID 由 Microsoft 账户绑定，无法重置' : ''}
            >
              重置 UUID
            </button>
            {selectedProfile.auth_method === 'Microsoft' && (
              <span
                className="auth-badge online"
                style={{
                  fontSize: '0.7rem',
                  padding: '4px 8px',
                  background: 'rgba(239, 68, 68, 0.15)',
                  color: '#ef4444',
                  border: '1px solid rgba(239, 68, 68, 0.3)',
                  borderRadius: 'var(--radius-sm)',
                  cursor: 'default',
                }}
              >
                在线账户 UUID 不可重置
              </span>
            )}
          </div>
        </div>
      )}
    </section>
  )
}
