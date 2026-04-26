import { useAppState } from '../hooks/useAppState'

export function Header() {
  const { uiMode, handleUiModeChange } = useAppState()

  return (
    <header className="header">
      <div className="header-brand">
        <h1>Obsidian<span>-</span>Launcher</h1>
        <span className="header-subtitle">Minecraft Java Edition</span>
      </div>
      <div className="ui-mode-toggle">
        <button
          className={`mode-btn ${uiMode === 'performance' ? 'active' : ''}`}
          onClick={() => handleUiModeChange('performance')}
        >
          性能
        </button>
        <button
          className={`mode-btn ${uiMode === 'gorgeous' ? 'active' : ''}`}
          onClick={() => handleUiModeChange('gorgeous')}
        >
          华丽
        </button>
      </div>
    </header>
  )
}
