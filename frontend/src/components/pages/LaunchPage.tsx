import { useEffect } from 'react'
import { motion } from 'framer-motion'
import { PageContainer } from '../layout/PageContainer'
import { VersionSelector } from '../ui/VersionSelector'
import { ProfileSelector } from '../ui/ProfileSelector'
import { LaunchButton } from '../ui/LaunchButton'
import { useVersion } from '../../contexts/VersionContext'
import { useProfile } from '../../contexts/ProfileContext'
import { useConfig } from '../../contexts/ConfigContext'
import { useGame } from '../../contexts/GameContext'
import { useDownload } from '../../contexts/DownloadContext'
import { tauriInvoke } from '../../hooks/useTauriInvoke'

const CONCURRENCY_OPTIONS = [4, 8, 12, 16, 24, 32, 64, 128]

export function LaunchPage() {
  const { versions, selectedVersion, versionType, setSelectedVersion, setVersionType, loadVersions, getFilteredVersions } = useVersion()
  const { profiles, selectedProfile, selectProfile, loadProfiles } = useProfile()
  const {
    javaList, selectedJava, setSelectedJava,
    concurrency, setConcurrency,
    customConcurrency, setCustomConcurrency,
    isCustomConcurrency, setIsCustomConcurrency,
    gameDir, savesIsolation, loadConfig
  } = useConfig()
  const { launching, setLaunching, setGameRunning } = useGame()
  const { downloading, downloadProgress } = useDownload()

  useEffect(() => {
    loadVersions()
    loadProfiles()
    loadConfig()
  }, [])

  const filteredVersions = versions
    .filter(v => {
      if (versionType === 'release') return v.type === 'release'
      if (versionType === 'snapshot') return v.type === 'snapshot'
      if (versionType === 'old') return v.type === 'old_alpha' || v.type === 'old_beta'
      return false
    })
    .map(v => v.id)

  async function handleLaunch() {
    if (!selectedVersion) return
    if (!selectedJava) return
    if (!selectedProfile) return

    setLaunching(true)
    setGameRunning(true)
    try {
      if (savesIsolation) {
        await tauriInvoke('prepare_saves_isolation', { version: selectedVersion })
      }
      await tauriInvoke('launch', {
        opts: {
          version: selectedVersion,
          game_dir: gameDir,
          username: selectedProfile.name,
          uuid: selectedProfile.uuid,
          runtime_dir: selectedJava,
          auth_method: selectedProfile.auth_method,
          access_token: selectedProfile.access_token ?? null
        }
      })
    } catch (e) {
      console.error('Launch failed:', e)
    } finally {
      setLaunching(false)
      setGameRunning(false)
    }
  }

  async function handleDownload() {
    if (!selectedVersion) return
    try {
      if (savesIsolation) {
        await tauriInvoke('prepare_saves_isolation', { version: selectedVersion })
      }
      const finalConcurrency = isCustomConcurrency ? parseInt(customConcurrency) || 32 : concurrency
      await tauriInvoke('download', { opts: { version: selectedVersion, game_dir: gameDir, concurrency: finalConcurrency } })
    } catch (e) {
      console.error('Download failed:', e)
    }
  }

  function handleVersionTabChange(type: 'release' | 'snapshot' | 'old') {
    setVersionType(type)
    const filtered = versions.filter(v => {
      if (type === 'release') return v.type === 'release'
      if (type === 'snapshot') return v.type === 'snapshot'
      if (type === 'old') return v.type === 'old_alpha' || v.type === 'old_beta'
      return true
    })
    if (filtered.length > 0) {
      setSelectedVersion(filtered[0].id)
    }
  }

  return (
    <PageContainer title="启动游戏">
      <div style={{
        display: 'grid',
        gridTemplateColumns: 'repeat(auto-fit, minmax(300px, 1fr))',
        gap: '20px',
      }}>
        <motion.div
          variants={{
            animate: { transition: { staggerChildren: 0.1 } },
          }}
        >
          <motion.div variants={{ initial: { opacity: 0, y: 20 }, animate: { opacity: 1, y: 0 } }}>
            <VersionSelector
              activeTab={versionType}
              onTabChange={handleVersionTabChange}
              versions={filteredVersions}
              selectedVersion={selectedVersion}
              onVersionChange={setSelectedVersion}
            />
          </motion.div>

          <motion.div variants={{ initial: { opacity: 0, y: 20 }, animate: { opacity: 1, y: 0 } }} style={{ marginTop: '20px' }}>
            <ProfileSelector
              profiles={profiles}
              selectedProfile={selectedProfile}
              onSelect={selectProfile}
            />
          </motion.div>
        </motion.div>

        <motion.div variants={{ initial: { opacity: 0, y: 20 }, animate: { opacity: 1, y: 0 } }}>
          <div className="card" style={{ padding: '20px', height: '100%' }}>
            <h3 style={{
              fontFamily: 'var(--font-display)',
              fontSize: '0.8rem',
              color: 'var(--text-muted)',
              textTransform: 'uppercase',
              letterSpacing: '0.1em',
              marginBottom: '16px',
            }}>
              游戏设置
            </h3>
            <div style={{ display: 'flex', flexDirection: 'column', gap: '12px' }}>
              <div>
                <label style={{ fontSize: '0.8rem', color: 'var(--text-muted)' }}>Java 运行时</label>
                <select
                  className="input select"
                  style={{ marginTop: '4px' }}
                  value={selectedJava}
                  onChange={(e) => setSelectedJava(e.target.value)}
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
              <div>
                <label style={{ fontSize: '0.8rem', color: 'var(--text-muted)' }}>并发下载数</label>
                <select
                  className="input select"
                  style={{ marginTop: '4px' }}
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
              </div>
              {isCustomConcurrency && (
                <input
                  type="number"
                  className="input"
                  placeholder="1-4096"
                  value={customConcurrency}
                  onChange={(e) => setCustomConcurrency(e.target.value)}
                />
              )}
            </div>
          </div>
        </motion.div>
      </div>

      {/* Download Progress */}
      {downloading && downloadProgress && (
        <motion.div
          initial={{ opacity: 0, y: 20 }}
          animate={{ opacity: 1, y: 0 }}
          style={{ marginTop: '24px' }}
        >
          <div className="card" style={{ padding: '20px' }}>
            <div style={{
              display: 'flex',
              justifyContent: 'space-between',
              marginBottom: '8px',
              fontSize: '0.8rem',
              color: 'var(--text-muted)',
            }}>
              <span>{downloadProgress.current_file}</span>
              <span>{downloadProgress.percent}%</span>
            </div>
            <div className="progress-bar">
              <div className="progress-fill" style={{ width: `${downloadProgress.percent}%` }} />
            </div>
          </div>
        </motion.div>
      )}

      <motion.div
        variants={{ initial: { opacity: 0, y: 20 }, animate: { opacity: 1, y: 0 } }}
        style={{ marginTop: '24px', display: 'flex', gap: '12px' }}
      >
        <div style={{ flex: 1 }}>
          <LaunchButton
            onClick={handleLaunch}
            disabled={!selectedVersion || !selectedJava || !selectedProfile}
            loading={launching}
          />
        </div>
        <button
          className="btn btn-secondary"
          onClick={handleDownload}
          disabled={downloading || !selectedVersion}
        >
          {downloading ? '下载中...' : '下载'}
        </button>
      </motion.div>
    </PageContainer>
  )
}
