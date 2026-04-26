import { motion } from 'framer-motion'
import { PageContainer } from '../layout/PageContainer'

export function SettingsPage() {
  return (
    <PageContainer title="设置">
      <div style={{ display: 'flex', flexDirection: 'column', gap: '20px', maxWidth: '600px' }}>
        <motion.div variants={{ initial: { opacity: 0, y: 20 }, animate: { opacity: 1, y: 0 } }}>
          <div className="glass-card" style={{ padding: '20px' }}>
            <h3 style={{
              fontFamily: 'var(--font-display)',
              fontSize: '0.8rem',
              color: 'var(--text-muted)',
              textTransform: 'uppercase',
              letterSpacing: '0.1em',
              marginBottom: '16px',
            }}>
              目录配置
            </h3>
            <div style={{ display: 'flex', flexDirection: 'column', gap: '12px' }}>
              <div>
                <label style={{ fontSize: '0.8rem', color: 'var(--text-muted)', display: 'block', marginBottom: '4px' }}>
                  根目录
                </label>
                <div style={{ display: 'flex', gap: '8px' }}>
                  <input
                    type="text"
                    className="input"
                    placeholder="/path/to/games"
                    style={{ flex: 1 }}
                    readOnly
                  />
                  <button className="btn btn-secondary">浏览</button>
                </div>
              </div>
            </div>
          </div>
        </motion.div>

        <motion.div variants={{ initial: { opacity: 0, y: 20 }, animate: { opacity: 1, y: 0 } }}>
          <div className="glass-card" style={{ padding: '20px' }}>
            <h3 style={{
              fontFamily: 'var(--font-display)',
              fontSize: '0.8rem',
              color: 'var(--text-muted)',
              textTransform: 'uppercase',
              letterSpacing: '0.1em',
              marginBottom: '16px',
            }}>
              隔离设置
            </h3>
            <div style={{ display: 'flex', flexDirection: 'column', gap: '12px' }}>
              <label style={{ display: 'flex', alignItems: 'center', gap: '12px', cursor: 'pointer' }}>
                <input type="checkbox" style={{ width: '18px', height: '18px', accentColor: 'var(--accent-primary)' }} />
                <div>
                  <p style={{ color: 'var(--text-primary)', fontWeight: 500 }}>版本隔离</p>
                  <p style={{ color: 'var(--text-muted)', fontSize: '0.8rem' }}>每个版本独立目录</p>
                </div>
              </label>
              <label style={{ display: 'flex', alignItems: 'center', gap: '12px', cursor: 'pointer' }}>
                <input type="checkbox" style={{ width: '18px', height: '18px', accentColor: 'var(--accent-primary)' }} />
                <div>
                  <p style={{ color: 'var(--text-primary)', fontWeight: 500 }}>存档隔离</p>
                  <p style={{ color: 'var(--text-muted)', fontSize: '0.8rem' }}>每版本独立存档</p>
                </div>
              </label>
            </div>
          </div>
        </motion.div>

        <motion.div variants={{ initial: { opacity: 0, y: 20 }, animate: { opacity: 1, y: 0 } }}>
          <div className="glass-card" style={{ padding: '20px' }}>
            <h3 style={{
              fontFamily: 'var(--font-display)',
              fontSize: '0.8rem',
              color: 'var(--text-muted)',
              textTransform: 'uppercase',
              letterSpacing: '0.1em',
              marginBottom: '16px',
            }}>
              界面模式
            </h3>
            <div style={{
              display: 'flex',
              gap: '8px',
              background: 'rgba(30, 27, 75, 0.4)',
              padding: '4px',
              borderRadius: 'var(--radius-md)',
            }}>
              <button
                className="btn"
                style={{
                  flex: 1,
                  background: 'var(--bg-card)',
                  color: 'var(--accent-primary)',
                }}
              >
                华丽模式
              </button>
              <button
                className="btn"
                style={{
                  flex: 1,
                  background: 'transparent',
                  color: 'var(--text-muted)',
                }}
              >
                性能模式
              </button>
            </div>
          </div>
        </motion.div>

        <motion.div variants={{ initial: { opacity: 0, y: 20 }, animate: { opacity: 1, y: 0 } }}>
          <div className="glass-card" style={{
            padding: '20px',
            borderColor: 'rgba(239, 68, 68, 0.3)',
          }}>
            <h3 style={{
              fontFamily: 'var(--font-display)',
              fontSize: '0.8rem',
              color: '#ef4444',
              textTransform: 'uppercase',
              letterSpacing: '0.1em',
              marginBottom: '16px',
            }}>
              危险区域
            </h3>
            <div style={{ display: 'flex', flexDirection: 'column', gap: '12px' }}>
              <button className="btn btn-danger">清空游戏文件</button>
              <button className="btn btn-danger">重置所有配置</button>
            </div>
          </div>
        </motion.div>
      </div>
    </PageContainer>
  )
}
