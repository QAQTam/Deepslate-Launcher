import { useAppState } from '../hooks/useAppState'
import { open } from '@tauri-apps/plugin-dialog'

const CONCURRENCY_OPTIONS = [4, 8, 12, 16, 24, 32, 64, 128]

export function SetupModal() {
  const {
    showSetup, setupStep, setSetupStep,
    setupRootDir, setSetupRootDir,
    setupVersionIsolation, setSetupVersionIsolation,
    setupJava, setSetupJava,
    javaList, selectedJava,
    concurrency, setConcurrency,
    customConcurrency, setCustomConcurrency,
    isCustomConcurrency, setIsCustomConcurrency,
    handleSetupComplete,
    rootDir, setShowSetup
  } = useAppState()

  if (!showSetup) return null

  return (
    <div className="modal-overlay">
      <div className="modal setup-modal">
        <h2>首次启动配置</h2>
        <div className="setup-progress">
          <div className={`setup-step ${setupStep >= 1 ? 'active' : ''} ${setupStep > 1 ? 'done' : ''}`} />
          <div className={`setup-step ${setupStep >= 2 ? 'active' : ''} ${setupStep > 2 ? 'done' : ''}`} />
        </div>

        {setupStep === 1 && (
          <div className="setup-step-content">
            <div className="setup-step-label">目录设置</div>
            <div className="setup-step-subtitle">选择游戏数据的根目录位置</div>
            <div className="setup-content">
              <div className="setup-dir-row">
                <span className="dir-path">
                  {setupRootDir || rootDir || '未选择'}
                </span>
                <button onClick={async () => {
                  const selected = await open({ directory: true, title: '选择根目录' })
                  if (selected) setSetupRootDir(selected as string)
                }} className="btn-sm btn-secondary">浏览</button>
              </div>
              <div className="setup-iso-card">
                <div className="setup-iso-header">
                  <input
                    type="checkbox"
                    id="setup-iso"
                    checked={setupVersionIsolation}
                    onChange={(e) => setSetupVersionIsolation(e.target.checked)}
                  />
                  <label htmlFor="setup-iso">启用版本隔离</label>
                </div>
                <p className="setup-iso-desc">
                  {setupVersionIsolation
                    ? '每版本独立根目录，游戏文件与存档各自隔离'
                    : '统一 game/ 目录管理，多版本共享文件'}
                </p>
              </div>
            </div>
            <div className="modal-actions">
              <button onClick={() => setShowSetup(false)} className="btn-ghost">跳过</button>
              <button onClick={() => setSetupStep(2)} className="btn-primary" disabled={!setupRootDir && !rootDir}>下一步</button>
            </div>
          </div>
        )}

        {setupStep === 2 && (
          <div className="setup-step-content">
            <div className="setup-step-label">运行环境</div>
            <div className="setup-step-subtitle">配置 Java 运行时与下载参数</div>
            <div className="setup-content">
              <div className="setup-field">
                <label className="setup-field-label">Java 版本</label>
                <select
                  value={setupJava || selectedJava}
                  onChange={e => setSetupJava(e.target.value)}
                >
                  {javaList.length === 0 ? (
                    <option value="">未检测到 Java</option>
                  ) : (
                    javaList.map(j => (
                      <option key={j.path} value={j.path}>{j.version}</option>
                    ))
                  )}
                </select>
              </div>
              <div className="setup-field">
                <label className="setup-field-label">下载并发数</label>
                <div className="concurrency-row">
                  <select
                    value={isCustomConcurrency ? 'custom' : concurrency}
                    onChange={(e) => {
                      if (e.target.value === 'custom') {
                        setIsCustomConcurrency(true)
                      } else {
                        setIsCustomConcurrency(false)
                        setConcurrency(parseInt(e.target.value))
                      }
                    }}
                  >
                    {CONCURRENCY_OPTIONS.map(n => (
                      <option key={n} value={n}>{n}</option>
                    ))}
                    <option value="custom">自定义</option>
                  </select>
                  {isCustomConcurrency && (
                    <input
                      type="number"
                      min={1}
                      max={4096}
                      value={customConcurrency}
                      onChange={(e) => {
                        const v = parseInt(e.target.value)
                        if (!isNaN(v) && v > 0 && v <= 4096) {
                          setCustomConcurrency(String(v))
                        } else if (e.target.value === '') {
                          setCustomConcurrency('')
                        }
                      }}
                      placeholder="1-4096"
                    />
                  )}
                </div>
              </div>
            </div>
            <div className="modal-actions">
              <button onClick={() => setSetupStep(1)} className="btn-ghost">上一步</button>
              <button onClick={handleSetupComplete} className="btn-primary">完成</button>
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
