import { useAppState } from '../hooks/useAppState'

export function LaunchSection() {
  const { handleLaunch, handleKill, launching, gameRunning } = useAppState()

  return (
    <section className="section">
      <h2>启动游戏</h2>
      <div className="btn-group">
        <button
          onClick={handleLaunch}
          disabled={launching}
          className="btn-primary"
        >
          {launching ? '启动中...' : '启动游戏'}
        </button>
        <button
          onClick={handleKill}
          disabled={!gameRunning}
          className="btn-danger"
        >
          终止游戏
        </button>
      </div>
    </section>
  )
}
