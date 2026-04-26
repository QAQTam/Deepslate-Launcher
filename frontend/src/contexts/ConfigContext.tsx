import { createContext, useContext, useState, useCallback, type ReactNode } from 'react'
import { open, ask } from '@tauri-apps/plugin-dialog'
import { tauriInvoke } from '../hooks/useTauriInvoke'

const CONCURRENCY_OPTIONS = [4, 8, 12, 16, 24, 32, 64, 128]

interface AppConfig {
  version: number
  root_dir: string
  version_isolation: boolean
  saves_isolation: boolean
  last_profile: string | null
  java_path: string | null
  download_concurrency: number
  first_launch: boolean
  ui_mode: string
}

interface ConfigContextType {
  // Directory state
  rootDir: string
  gameDir: string
  savesDir: string
  versionsDir: string
  versionIsolation: boolean
  savesIsolation: boolean
  // Java state
  javaList: { path: string; version: string }[]
  selectedJava: string
  // UI
  uiMode: 'performance' | 'gorgeous'
  status: string
  // Concurrency
  concurrency: number
  customConcurrency: string
  isCustomConcurrency: boolean
  // Actions
  loadConfig: () => Promise<void>
  setSelectedJava: (path: string) => void
  setVersionIsolation: (v: boolean) => void
  setSavesIsolation: (v: boolean) => void
  selectRootDir: () => Promise<void>
  handleSavePaths: () => Promise<void>
  handleUiModeChange: (mode: 'performance' | 'gorgeous') => void
  handleClearGameFiles: () => Promise<void>
  handleResetAllConfig: () => Promise<void>
  handleDeleteUserConfig: () => Promise<void>
  scanJavaDir: (dir: string) => Promise<void>
  setConcurrency: (v: number) => void
  setCustomConcurrency: (v: string) => void
  setIsCustomConcurrency: (v: boolean) => void
  setStatus: (s: string) => void
  updateDirsFromRoot: (root: string, iso: boolean) => void
}

const ConfigContext = createContext<ConfigContextType | null>(null)

export function ConfigProvider({ children }: { children: ReactNode }) {
  const [rootDir, setRootDir] = useState('')
  const [gameDir, setGameDir] = useState('')
  const [savesDir, setSavesDir] = useState('')
  const [versionsDir, setVersionsDir] = useState('')
  const [versionIsolation, setVersionIsolation] = useState(false)
  const [savesIsolation, setSavesIsolation] = useState(false)
  const [javaList, setJavaList] = useState<{ path: string; version: string }[]>([])
  const [selectedJava, setSelectedJava] = useState('')
  const [uiMode, setUiMode] = useState<'performance' | 'gorgeous'>('gorgeous')
  const [status, setStatus] = useState('就绪')
  const [concurrency, setConcurrency] = useState(32)
  const [customConcurrency, setCustomConcurrency] = useState('')
  const [isCustomConcurrency, setIsCustomConcurrency] = useState(false)

  const updateDirsFromRoot = useCallback((root: string, iso: boolean) => {
    if (iso) {
      setGameDir(root)
      setVersionsDir(`${root}/versions`)
    } else {
      setGameDir(`${root}/game`)
      setVersionsDir(`${root}/game/versions`)
    }
    setSavesDir(`${root}/saves`)
  }, [])

  const loadConfig = useCallback(async () => {
    try {
      const config = await tauriInvoke<AppConfig>('get_config')
      if (config.root_dir) {
        setRootDir(config.root_dir)
        setVersionIsolation(config.version_isolation)
        setSavesIsolation(config.saves_isolation)
        updateDirsFromRoot(config.root_dir, config.version_isolation)
      }
      if (config.java_path) setSelectedJava(config.java_path)
      if (config.download_concurrency) {
        const val = config.download_concurrency
        if (CONCURRENCY_OPTIONS.includes(val)) {
          setConcurrency(val)
          setIsCustomConcurrency(false)
        } else {
          setConcurrency(0)
          setCustomConcurrency(String(val))
          setIsCustomConcurrency(true)
        }
      }
      if (config.ui_mode === 'performance' || config.ui_mode === 'gorgeous') {
        setUiMode(config.ui_mode as 'performance' | 'gorgeous')
        document.documentElement.setAttribute('data-ui-mode', config.ui_mode)
      }
    } catch (e) {
      console.error('Failed to load config:', e)
    }
  }, [updateDirsFromRoot])

  const selectRootDir = useCallback(async () => {
    try {
      const selected = await open({ directory: true, multiple: false, title: '选择根目录' })
      if (selected) {
        const dir = selected as string
        setRootDir(dir)
        updateDirsFromRoot(dir, versionIsolation)
        setStatus(`已选择根目录: ${dir}`)
      }
    } catch (e) {
      setStatus(`选择目录失败: ${e}`)
    }
  }, [versionIsolation, updateDirsFromRoot])

  const handleSavePaths = useCallback(async () => {
    try {
      const config = await tauriInvoke<AppConfig>('get_config')
      await tauriInvoke('set_config', {
        config: {
          ...config,
          root_dir: rootDir,
          version_isolation: versionIsolation,
          saves_isolation: versionIsolation ? false : savesIsolation,
        }
      })
      setStatus('目录配置已保存')
    } catch (e) {
      setStatus(`保存失败: ${e}`)
    }
  }, [rootDir, versionIsolation, savesIsolation])

  const handleUiModeChange = useCallback((mode: 'performance' | 'gorgeous') => {
    setUiMode(mode)
    document.documentElement.setAttribute('data-ui-mode', mode)
    tauriInvoke('set_config', { config: { ui_mode: mode } }).catch(console.error)
  }, [])

  const handleClearGameFiles = useCallback(async () => {
    if (!gameDir) { setStatus('游戏目录未设置'); return }
    const msg = ['确定清空游戏目录吗？', '', `目录: ${gameDir}`, '', '将删除:', '  - versions/ (所有版本)', '  - libraries/ (所有依赖库)', '  - assets/ (所有资源文件)', '', '目录本身将保留', '此操作不可逆！'].join('\n')
    if (!await ask(msg, { title: '清空游戏文件', kind: 'warning' })) return
    if (!await ask('再次确认：清空所有游戏文件？', { title: '二次确认', kind: 'warning' })) return
    try {
      await tauriInvoke('clear_game_files', { gameDir })
      setStatus('游戏文件已清空')
    } catch (e) {
      setStatus(`清空失败: ${e}`)
    }
  }, [gameDir])

  const handleDeleteUserConfig = useCallback(async () => {
    const msg = ['确定删除用户配置吗？', '', '将删除:', '  - 配置文件 (config.json)', '  - 游戏目录设置', '  - 下载并发数设置', '  - UI 模式设置', '', '档案 (profiles.json) 将保留'].join('\n')
    if (!await ask(msg, { title: '删除用户配置', kind: 'warning' })) return
    try {
      await tauriInvoke('delete_user_config')
      setRootDir(''); setGameDir(''); setSavesDir(''); setVersionsDir('')
      setSelectedJava(''); setConcurrency(32); setIsCustomConcurrency(false); setCustomConcurrency('')
      await loadConfig()
      setStatus('用户配置已重置，需重新设置')
    } catch (e) {
      setStatus(`删除失败: ${e}`)
    }
  }, [loadConfig])

  const handleResetAllConfig = useCallback(async () => {
    const msg = ['确定删除所有配置并重新初始化吗？', '', '将删除:', '  - config.json (所有配置)', '  - profiles.json (所有用户档案)', '', '游戏文件不会被删除', '此操作不可逆！'].join('\n')
    if (!await ask(msg, { title: '重置所有配置', kind: 'warning' })) return
    if (!await ask('再次确认：删除所有配置？', { title: '二次确认', kind: 'warning' })) return
    try {
      await tauriInvoke('reset_all_config')
      setRootDir(''); setGameDir(''); setSavesDir(''); setVersionsDir('')
      setSelectedJava(''); setConcurrency(32); setIsCustomConcurrency(false); setCustomConcurrency('')
      await loadConfig()
      setStatus('所有配置已重置，正在重新初始化...')
    } catch (e) {
      setStatus(`重置失败: ${e}`)
    }
  }, [loadConfig])

  const scanJavaDir = useCallback(async (dir: string) => {
    setStatus(`正在扫描: ${dir}`)
    try {
      const javas = await tauriInvoke<{ path: string; version: string }[]>('scan_java_dir', { dir })
      if (javas.length > 0) {
        setJavaList(prev => {
          const filtered = prev.filter(j => !javas.some((n: { path: string; version: string }) => n.path === j.path))
          return [...javas, ...filtered]
        })
        setSelectedJava(javas[0].path)
        setStatus(`找到 ${javas.length} 个 Java 运行时`)
      } else {
        setStatus('未在该目录找到 Java 运行时')
      }
    } catch (e) {
      setStatus(`扫描失败: ${e}`)
    }
  }, [])

  return (
    <ConfigContext.Provider value={{
      rootDir, gameDir, savesDir, versionsDir, versionIsolation, savesIsolation,
      javaList, selectedJava, uiMode, status,
      concurrency, customConcurrency, isCustomConcurrency,
      loadConfig, setSelectedJava, setVersionIsolation, setSavesIsolation,
      selectRootDir, handleSavePaths, handleUiModeChange,
      handleClearGameFiles, handleResetAllConfig, handleDeleteUserConfig,
      scanJavaDir, setConcurrency, setCustomConcurrency, setIsCustomConcurrency,
      setStatus, updateDirsFromRoot
    }}>
      {children}
    </ConfigContext.Provider>
  )
}

export function useConfig() {
  const ctx = useContext(ConfigContext)
  if (!ctx) throw new Error('useConfig must be used within ConfigProvider')
  return ctx
}