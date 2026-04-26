import { useAppState } from '../hooks/useAppState'
import { open } from '@tauri-apps/plugin-dialog'

export function JavaSection() {
  const { javaList, selectedJava, setSelectedJava, loadJava, launching, setStatus, scanJavaDir } = useAppState()

  async function handleScanCustomDir() {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: '选择 Java 运行时目录'
      })
      if (selected) {
        await scanJavaDir(selected as string)
      }
    } catch (e) {
      setStatus(`扫描失败: ${e}`)
    }
  }

  return (
    <section className="section">
      <h2>选择 Java</h2>
      <select
        value={selectedJava}
        onChange={(e) => setSelectedJava(e.target.value)}
        disabled={launching}
      >
        {javaList.length === 0 ? (
          <option value="">未检测到 Java</option>
        ) : (
          javaList.map(j => (
            <option key={j.path} value={j.path}>
              {j.version} - {j.path}
            </option>
          ))
        )}
      </select>
      <button onClick={loadJava} className="btn-secondary" style={{ marginTop: '8px', width: '100%' }}>
        重新扫描
      </button>
      <button
        onClick={handleScanCustomDir}
        className="btn-secondary"
        style={{ marginTop: '8px', width: '100%' }}
      >
        扫描自定义目录
      </button>
    </section>
  )
}
