import { useAppState } from '../hooks/useAppState'

export function DirectorySection() {
  const {
    rootDir, versionsDir, savesDir,
    versionIsolation, setVersionIsolation,
    savesIsolation, setSavesIsolation,
    selectRootDir, handleSavePaths, handleClearGameFiles
  } = useAppState()

  function handleVersionIsolationToggle(iso: boolean) {
    setVersionIsolation(iso)
  }

  return (
    <section className="section">
      <h2>目录设置</h2>
      <div className="dir-row" style={{ marginBottom: '8px' }}>
        <span className="dir-path" title={rootDir}>{rootDir || '未设置'}</span>
        <button onClick={selectRootDir} className="btn-sm btn-secondary">根目录</button>
      </div>
      <div className="dir-row" style={{ marginBottom: '8px' }}>
        <span className="dir-path" title={versionsDir}>{versionsDir || '未设置'}</span>
        <span style={{ fontSize: '0.7rem', color: 'var(--text-muted)', fontFamily: 'var(--font-mono)', padding: '4px 8px' }}>版本目录</span>
      </div>
      <div className="dir-row" style={{ marginBottom: '12px' }}>
        <span className="dir-path" title={savesDir}>{savesDir || '未设置'}</span>
        <span style={{ fontSize: '0.7rem', color: 'var(--text-muted)', fontFamily: 'var(--font-mono)', padding: '4px 8px' }}>存档目录</span>
      </div>
      <div style={{ display: 'flex', alignItems: 'center', gap: '12px', marginBottom: '8px', padding: '8px 12px', background: 'var(--bg-secondary)', borderRadius: 'var(--radius-sm)', border: '1px solid var(--border-subtle)' }}>
        <label style={{ display: 'flex', alignItems: 'center', gap: '8px', cursor: 'pointer', flex: 1 }}>
          <input
            type="checkbox"
            checked={versionIsolation}
            onChange={(e) => handleVersionIsolationToggle(e.target.checked)}
            style={{ width: '16px', height: '16px', accentColor: 'var(--accent)' }}
          />
          <span style={{ fontSize: '0.85rem', color: 'var(--text-primary)' }}>版本隔离</span>
        </label>
        <span style={{ fontSize: '0.75rem', color: 'var(--text-muted)', fontFamily: 'var(--font-mono)' }}>
          {versionIsolation ? '每版本完整隔离' : '统一游戏目录'}
        </span>
      </div>
      <div style={{ display: 'flex', alignItems: 'center', gap: '12px', marginBottom: '12px', padding: '8px 12px', background: 'var(--bg-secondary)', borderRadius: 'var(--radius-sm)', border: '1px solid var(--accent)', borderColor: versionIsolation ? 'var(--border-subtle)' : 'rgba(239, 68, 68, 0.3)' }}>
        <label style={{ display: 'flex', alignItems: 'center', gap: '8px', cursor: versionIsolation ? 'not-allowed' : 'pointer', flex: 1, opacity: versionIsolation ? 0.4 : 1 }}>
          <input
            type="checkbox"
            checked={versionIsolation ? false : savesIsolation}
            onChange={(e) => !versionIsolation && setSavesIsolation(e.target.checked)}
            disabled={versionIsolation}
            style={{ width: '16px', height: '16px', accentColor: 'var(--accent)' }}
          />
          <span style={{ fontSize: '0.85rem', color: 'var(--text-primary)' }}>
            隔离存档 <span style={{ fontSize: '0.7rem', color: 'var(--warning)' }}>⚠ 实验级</span>
          </span>
        </label>
        <span style={{ fontSize: '0.75rem', color: 'var(--text-muted)', fontFamily: 'var(--font-mono)' }}>
          {versionIsolation ? '由版本隔离覆盖' : (savesIsolation ? '每版本独立存档' : '统一存档')}
        </span>
      </div>
      <div className="btn-group">
        <button onClick={handleSavePaths} className="btn-primary">保存配置</button>
        <button onClick={handleClearGameFiles} className="btn-danger">清空游戏文件</button>
      </div>
    </section>
  )
}
