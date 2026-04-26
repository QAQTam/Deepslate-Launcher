import { useAppState } from '../hooks/useAppState'

export function ProfileModal() {
  const {
    showProfileModal, setShowProfileModal,
    isLoggingIn, loginMessage, microsoftCodeInput, setMicrosoftCodeInput,
    newProfileName, setNewProfileName,
    handleOpenMicrosoftAuth, handleMicrosoftWithCode, handleCreateProfile
  } = useAppState()

  if (!showProfileModal) return null

  return (
    <div className="modal-overlay" onClick={() => setShowProfileModal(false)}>
      <div className="modal" onClick={e => e.stopPropagation()}>
        <h3>新建用户档案</h3>
        {!isLoggingIn ? (
          <>
            <p className="modal-hint">使用 Microsoft 账户登录在线模式</p>
            <button
              onClick={handleOpenMicrosoftAuth}
              className="btn-primary"
              style={{ marginBottom: '16px', background: '#0078d4' }}
            >
              1. 打开授权页面
            </button>
            <p className="modal-hint" style={{ fontSize: '0.8rem', marginBottom: '8px' }}>
              登录成功后，复制浏览器地址栏的完整 URL 粘贴到下方
            </p>
            <input
              type="text"
              value={microsoftCodeInput}
              onChange={(e) => setMicrosoftCodeInput(e.target.value)}
              placeholder="粘贴回调 URL（包含 code=）"
              style={{ marginBottom: '8px' }}
            />
            <button
              onClick={handleMicrosoftWithCode}
              className="btn-primary"
              style={{ marginBottom: '16px', background: '#107C10' }}
              disabled={!microsoftCodeInput.trim()}
            >
              2. 完成登录
            </button>
            <div className="divider">或</div>
            <p className="modal-hint">档案将使用基于用户名的离线 UUID</p>
            <input
              type="text"
              value={newProfileName}
              onChange={(e) => setNewProfileName(e.target.value)}
              placeholder="输入用户名"
              onKeyDown={e => e.key === 'Enter' && handleCreateProfile()}
            />
            <div className="modal-actions">
              <button onClick={() => setShowProfileModal(false)} className="btn-ghost">取消</button>
              <button onClick={handleCreateProfile} className="btn-primary">创建离线档案</button>
            </div>
          </>
        ) : (
          <>
            <p className="modal-hint">正在登录 Microsoft 账户...</p>
            <div className="login-progress">
              <p>{loginMessage}</p>
            </div>
          </>
        )}
      </div>
    </div>
  )
}
