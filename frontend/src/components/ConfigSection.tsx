import { useAppState } from '../hooks/useAppState'

export function ConfigSection() {
  const { handleDeleteUserConfig, handleResetAllConfig } = useAppState()

  return (
    <section className="section danger-zone">
      <h2>配置管理</h2>
      <div className="btn-group">
        <button onClick={handleDeleteUserConfig} className="btn-secondary">
          删除用户配置
        </button>
        <button onClick={handleResetAllConfig} className="btn-danger">
          重置所有配置
        </button>
      </div>
    </section>
  )
}
