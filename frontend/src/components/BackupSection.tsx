import { useState, useEffect } from 'react'
import { tauriInvoke } from '../hooks/useTauriInvoke'

interface VersionInfo { id: string; type: string; url: string; time: string; releaseTime: string }

export function BackupSection() {
  const [versions, setVersions] = useState<VersionInfo[]>([])
  const [selectedVersion, setSelectedVersion] = useState('')
  const [backups, setBackups] = useState<string[]>([])
  const [message, setMessage] = useState('')
  const [loading, setLoading] = useState(false)

  // Load versions on mount
  useEffect(() => {
    tauriInvoke<VersionInfo[]>('get_versions')
      .then(list => {
        setVersions(list)
        if (list.length > 0 && !selectedVersion) {
          setSelectedVersion(list[0].id)
        }
      })
      .catch(e => setMessage(`加载版本列表失败: ${e}`))
  }, [])

  // Load backups when version changes
  useEffect(() => {
    if (!selectedVersion) return
    loadBackups()
  }, [selectedVersion])

  async function loadBackups() {
    try {
      const list = await tauriInvoke<string[]>('list_saves_backups', { version: selectedVersion })
      setBackups(list)
    } catch (e) {
      setMessage(`加载备份列表失败: ${e}`)
    }
  }

  async function handleCreateBackup() {
    setLoading(true)
    setMessage('')
    try {
      const name = await tauriInvoke<string>('create_saves_backup', { version: selectedVersion })
      setMessage(`备份创建成功: ${name}`)
      await loadBackups()
    } catch (e) {
      setMessage(`创建备份失败: ${e}`)
    } finally {
      setLoading(false)
    }
  }

  async function handleRestore(backupName: string) {
    setLoading(true)
    setMessage('')
    try {
      await tauriInvoke('restore_saves_backup', { version: selectedVersion, backupName })
      setMessage(`备份 "${backupName}" 恢复成功`)
    } catch (e) {
      setMessage(`恢复失败: ${e}`)
    } finally {
      setLoading(false)
    }
  }

  async function handleDelete(backupName: string) {
    setLoading(true)
    setMessage('')
    try {
      await tauriInvoke('delete_saves_backup', { version: selectedVersion, backupName })
      setMessage(`备份 "${backupName}" 已删除`)
      await loadBackups()
    } catch (e) {
      setMessage(`删除失败: ${e}`)
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="card" style={{ marginBottom: 'var(--space-lg)' }}>
      <h3 style={{ fontFamily: 'var(--font-display)', fontWeight: 700, marginBottom: 'var(--space-md)' }}>
        存档备份
      </h3>

      {/* Version selector */}
      <div style={{ marginBottom: 'var(--space-md)' }}>
        <label style={{ fontSize: '0.85rem', color: 'var(--text-secondary)', marginBottom: '4px', display: 'block' }}>
          目标版本
        </label>
        <select
          value={selectedVersion}
          onChange={e => setSelectedVersion(e.target.value)}
          style={{
            width: '100%',
            maxWidth: '320px',
            padding: '8px 12px',
          }}
        >
          {versions.map((v) => (
            <option key={v.id} value={v.id}>{v.id}</option>
          ))}
        </select>
      </div>

      {/* Actions */}
      <div className="btn-group" style={{ marginBottom: 'var(--space-md)' }}>
        <button
          className="btn-primary"
          onClick={handleCreateBackup}
          disabled={loading || !selectedVersion}
        >
          创建备份
        </button>
      </div>

      {/* Message */}
      {message && (
        <div style={{
          padding: '8px 12px',
          borderRadius: 'var(--radius-sm)',
          background: message.includes('失败') ? 'var(--danger-light)' : 'var(--success-light)',
          color: message.includes('失败') ? 'var(--danger)' : 'var(--success)',
          fontSize: '0.85rem',
          marginBottom: 'var(--space-md)',
        }}>
          {message}
        </div>
      )}

      {/* Backup list */}
      {backups.length > 0 ? (
        <div style={{ display: 'flex', flexDirection: 'column', gap: '8px' }}>
          {backups.map(b => (
            <div
              key={b}
              className="card"
              style={{
                padding: '12px 16px',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'space-between',
              }}
            >
              <span style={{
                fontFamily: 'var(--font-mono)',
                fontSize: '0.85rem',
                color: 'var(--text-primary)',
              }}>
                {b}
              </span>
              <div style={{ display: 'flex', gap: '8px' }}>
                <button
                  className="btn-secondary btn-sm"
                  onClick={() => handleRestore(b)}
                  disabled={loading}
                >
                  恢复
                </button>
                <button
                  className="btn-danger btn-sm"
                  onClick={() => handleDelete(b)}
                  disabled={loading}
                >
                  删除
                </button>
              </div>
            </div>
          ))}
        </div>
      ) : (
        <p style={{ color: 'var(--text-muted)', fontSize: '0.85rem' }}>
          暂无备份。创建第一个备份以保护你的存档。
        </p>
      )}
    </div>
  )
}
