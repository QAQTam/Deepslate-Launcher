import { useState } from 'react'
import { motion } from 'framer-motion'
import { PageContainer } from '../layout/PageContainer'
import { VersionSelector } from '../ui/VersionSelector'
import { ProfileSelector } from '../ui/ProfileSelector'
import { LaunchButton } from '../ui/LaunchButton'

export function LaunchPage() {
  const [activeTab, setActiveTab] = useState<'release' | 'snapshot' | 'old'>('release')
  const [selectedVersion, setSelectedVersion] = useState('')

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
              activeTab={activeTab}
              onTabChange={setActiveTab}
              versions={['1.20.4', '1.20.2', '1.19.4']}
              selectedVersion={selectedVersion}
              onVersionChange={setSelectedVersion}
            />
          </motion.div>

          <motion.div variants={{ initial: { opacity: 0, y: 20 }, animate: { opacity: 1, y: 0 } }} style={{ marginTop: '20px' }}>
            <ProfileSelector
              profiles={[
                { name: 'Player123', uuid: 'abc-123', auth_method: 'Offline' },
                { name: 'Steve', uuid: 'def-456', auth_method: 'Microsoft' },
              ]}
              selectedProfile={null}
              onSelect={() => {}}
            />
          </motion.div>
        </motion.div>

        <motion.div variants={{ initial: { opacity: 0, y: 20 }, animate: { opacity: 1, y: 0 } }}>
          <div className="glass-card" style={{ padding: '20px', height: '100%' }}>
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
                <select className="input select" style={{ marginTop: '4px' }}>
                  <option>Java 17</option>
                  <option>Java 21</option>
                </select>
              </div>
              <div>
                <label style={{ fontSize: '0.8rem', color: 'var(--text-muted)' }}>并发下载数</label>
                <select className="input select" style={{ marginTop: '4px' }}>
                  <option>32</option>
                  <option>16</option>
                  <option>8</option>
                </select>
              </div>
            </div>
          </div>
        </motion.div>
      </div>

      <motion.div
        variants={{ initial: { opacity: 0, y: 20 }, animate: { opacity: 1, y: 0 } }}
        style={{ marginTop: '24px' }}
      >
        <LaunchButton onClick={() => {}} disabled={!selectedVersion} loading={false} />
      </motion.div>
    </PageContainer>
  )
}
