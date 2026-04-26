import { useAppState } from '../hooks/useAppState'

const CONCURRENCY_OPTIONS = [4, 8, 12, 16, 24, 32, 64, 128]

export function DownloadSection() {
  const {
    downloading, downloadProgress,
    concurrency, setConcurrency,
    customConcurrency, setCustomConcurrency,
    isCustomConcurrency, setIsCustomConcurrency,
    handleDownload
  } = useAppState()

  return (
    <section className="section">
      <h2>下载游戏</h2>
      <div className="concurrency-row">
        <label>并发数:</label>
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
          disabled={downloading}
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
            disabled={downloading}
          />
        )}
      </div>
      {downloading && downloadProgress && (
        <div className="progress-container">
          <div className="progress-bar">
            <div className="progress-fill" style={{ width: `${downloadProgress.percent}%` }} />
          </div>
          <div className="progress-text">
            <span>{downloadProgress.current_file}</span>
            <span>{downloadProgress.percent}% ({downloadProgress.current}/{downloadProgress.total})</span>
          </div>
        </div>
      )}
      <button
        onClick={handleDownload}
        disabled={downloading}
        className="btn-primary"
      >
        {downloading ? '下载中...' : '下载'}
      </button>
    </section>
  )
}
