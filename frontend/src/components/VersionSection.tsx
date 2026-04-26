import { useAppState } from '../hooks/useAppState'

export function VersionSection() {
  const { versions, selectedVersion, versionType, setSelectedVersion, handleVersionTabChange } = useAppState()

  function getFilteredVersions() {
    return versions.filter((v) => {
      if (versionType === 'release') return v.type === 'release'
      if (versionType === 'snapshot') return v.type === 'snapshot'
      if (versionType === 'old') return v.type === 'old_alpha' || v.type === 'old_beta'
      return true
    })
  }

  return (
    <section className="section">
      <h2>选择版本</h2>
      <div className="version-tabs">
        <button
          className={`version-tab ${versionType === 'release' ? 'active' : ''}`}
          onClick={() => handleVersionTabChange('release')}
        >
          正式版
        </button>
        <button
          className={`version-tab ${versionType === 'snapshot' ? 'active' : ''}`}
          onClick={() => handleVersionTabChange('snapshot')}
        >
          测试版
        </button>
        <button
          className={`version-tab ${versionType === 'old' ? 'active' : ''}`}
          onClick={() => handleVersionTabChange('old')}
        >
          历史版
        </button>
      </div>
      <select
        value={selectedVersion}
        onChange={(e) => setSelectedVersion(e.target.value)}
      >
        {getFilteredVersions().map(v => (
          <option key={v.id} value={v.id}>{v.id}</option>
        ))}
      </select>
    </section>
  )
}
