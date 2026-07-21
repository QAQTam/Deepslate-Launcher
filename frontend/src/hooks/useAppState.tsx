import { createContext, useContext, useState, useEffect, useCallback, type ReactNode } from 'react'
import { open, ask } from '@tauri-apps/plugin-dialog'
import { listen } from '@tauri-apps/api/event'
import { tauriInvoke } from './useTauriInvoke'
import { useDownloadProgress } from './useDownloadProgress'
import { useGameState } from './useGameState'
import type { VersionInfo, JavaInfo, UserProfile, ProfileStore, AppConfig } from '../types'

const CONCURRENCY_OPTIONS = [4, 8, 12, 16, 24, 32, 64, 128]

interface AppState {
  // Version state
  versions: VersionInfo[]
  selectedVersion: string
  versionType: 'release' | 'snapshot' | 'old'
  // Java state
  javaList: JavaInfo[]
  selectedJava: string
  // Directory state
  rootDir: string
  gameDir: string
  savesDir: string
  versionsDir: string
  versionIsolation: boolean
  savesIsolation: boolean
  // Game state
  launching: boolean
  gameRunning: boolean
  status: string
  // Profile state
  profiles: UserProfile[]
  selectedProfile: UserProfile | null
  // Concurrency
  concurrency: number
  customConcurrency: string
  isCustomConcurrency: boolean
  // UI
  uiMode: 'performance' | 'gorgeous'
  showSetup: boolean
  setupStep: number
  setupRootDir: string
  setupVersionIsolation: boolean
  setupJava: string
  // Profile modal
  showProfileModal: boolean
  newProfileName: string
  isLoggingIn: boolean
  loginMessage: string
  microsoftCodeInput: string
  // Download progress
  downloadProgress: { current: number; total: number; percent: number; current_file: string } | null
  downloading: boolean
}

interface AppActions {
  setStatus: (s: string) => void
  loadVersions: () => Promise<void>
  loadJava: () => Promise<void>
  loadProfiles: () => Promise<void>
  loadConfig: () => Promise<void>
  setSelectedVersion: (v: string) => void
  setVersionType: (t: 'release' | 'snapshot' | 'old') => void
  setSelectedJava: (j: string) => void
  setRootDir: (d: string) => void
  setVersionIsolation: (v: boolean) => void
  setSavesIsolation: (v: boolean) => void
  handleDownload: () => Promise<void>
  handleLaunch: () => Promise<void>
  handleKill: () => Promise<void>
  handleCreateProfile: () => Promise<void>
  handleDeleteProfile: (name: string) => Promise<void>
  handleResetUuid: () => Promise<void>
  handleDeleteUserConfig: () => Promise<void>
  handleResetAllConfig: () => Promise<void>
  handleUiModeChange: (mode: 'performance' | 'gorgeous') => void
  handleClearGameFiles: () => Promise<void>
  handleSetupComplete: () => Promise<void>
  handleOpenMicrosoftAuth: () => Promise<void>
  handleMicrosoftWithCode: () => Promise<void>
  setShowProfileModal: (v: boolean) => void
  setNewProfileName: (n: string) => void
  setMicrosoftCodeInput: (c: string) => void
  setSetupStep: (s: number) => void
  setSetupRootDir: (d: string) => void
  setSetupVersionIsolation: (v: boolean) => void
  setSetupJava: (j: string) => void
  setShowSetup: (v: boolean) => void
  selectRootDir: () => Promise<void>
  handleSavePaths: () => Promise<void>
  handleVersionTabChange: (t: 'release' | 'snapshot' | 'old') => void
  scanJavaDir: (dir: string) => Promise<void>
  setDownloading: (v: boolean) => void
  getFilteredVersions: () => VersionInfo[]
  setSelectedProfile: (p: UserProfile | null) => void
  setConcurrency: (v: number) => void
  setCustomConcurrency: (v: string) => void
  setIsCustomConcurrency: (v: boolean) => void
}

type AppContextType = AppState & AppActions

const AppContext = createContext<AppContextType | null>(null)

export function AppProvider({ children }: { children: ReactNode }) {
  // Version state
  const [versions, setVersions] = useState<VersionInfo[]>([])
  const [selectedVersion, setSelectedVersion] = useState('')
  const [versionType, setVersionType] = useState<'release' | 'snapshot' | 'old'>('release')

  // Java state
  const [javaList, setJavaList] = useState<JavaInfo[]>([])
  const [selectedJava, setSelectedJava] = useState('')

  // Directory state
  const [rootDir, setRootDir] = useState('')
  const [gameDir, setGameDir] = useState('')
  const [savesDir, setSavesDir] = useState('')
  const [versionsDir, setVersionsDir] = useState('')
  const [versionIsolation, setVersionIsolation] = useState(false)
  const [savesIsolation, setSavesIsolation] = useState(false)

  // Game state
  const { launching, setLaunching, gameRunning, setGameRunning } = useGameState()
  const [status, setStatus] = useState('就绪')

  // Profile state
  const [profiles, setProfiles] = useState<UserProfile[]>([])
  const [selectedProfile, setSelectedProfile] = useState<UserProfile | null>(null)

  // Concurrency
  const [concurrency, setConcurrency] = useState(32)
  const [customConcurrency, setCustomConcurrency] = useState('')
  const [isCustomConcurrency, setIsCustomConcurrency] = useState(false)

  // UI
  const [uiMode, setUiMode] = useState<'performance' | 'gorgeous'>('gorgeous')
  const [showSetup, setShowSetup] = useState(false)
  const [setupStep, setSetupStep] = useState(1)
  const [setupRootDir, setSetupRootDir] = useState('')
  const [setupVersionIsolation, setSetupVersionIsolation] = useState(false)
  const [setupJava, setSetupJava] = useState('')

  // Profile modal
  const [showProfileModal, setShowProfileModal] = useState(false)
  const [newProfileName, setNewProfileName] = useState('')
  const [isLoggingIn, setIsLoggingIn] = useState(false)
  const [loginMessage, setLoginMessage] = useState('')
  const [microsoftCodeInput, setMicrosoftCodeInput] = useState('')

  // Download progress
  const { downloading, setDownloading, progress: downloadProgress } = useDownloadProgress()

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

  async function loadVersions() {
    try {
      const vers = await tauriInvoke<VersionInfo[]>('get_versions')
      setVersions(vers)
      const latestRelease = vers.find((v: VersionInfo) => v.type === 'release')
      if (latestRelease) {
        setSelectedVersion(latestRelease.id)
        setVersionType('release')
      }
    } catch (e) {
      setStatus(`加载版本失败: ${e}`)
    }
  }

  async function loadJava() {
    try {
      const javas = await tauriInvoke<JavaInfo[]>('scan_java')
      setJavaList(javas)
      if (javas.length > 0) {
        setSelectedJava(javas[0].path)
      }
    } catch (e) {
      console.error('扫描 Java 失败:', e)
    }
  }

  async function loadProfiles() {
    try {
      const store = await tauriInvoke<ProfileStore>('get_profiles')
      setProfiles(store.profiles)
      if (store.last_used) {
        const last = store.profiles.find((p: UserProfile) => p.name === store.last_used)
        if (last) setSelectedProfile(last)
      }
    } catch (e) {
      console.error('加载档案失败:', e)
    }
  }

  async function loadConfig() {
    try {
      const config = await tauriInvoke<AppConfig>('get_config')
      if (config.first_launch !== false) {
        setShowSetup(true)
      }
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
    } catch {
      setShowSetup(true)
    }
  }

  useEffect(() => {
    loadConfig()
    loadVersions()
    loadJava()
    loadProfiles()
  }, [])

  // Listen for auth progress from backend
  useEffect(() => {
    const setup = async () => {
      const unlisten = await listen<string>('auth-progress', (event) => {
        setStatus(`认证: ${event.payload}`)
      })
      return unlisten
    }
    let unlistenFn: (() => void) | null = null
    setup().then(fn => { unlistenFn = fn })
    return () => { unlistenFn?.() }
  }, [])

  async function selectRootDir() {
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
  }

  function handleVersionTabChange(type: 'release' | 'snapshot' | 'old') {
    setVersionType(type)
    if (type === 'release') {
      const latest = versions.find((v: VersionInfo) => v.type === 'release')
      if (latest) setSelectedVersion(latest.id)
    } else if (type === 'snapshot') {
      const latest = versions.find((v: VersionInfo) => v.type === 'snapshot')
      if (latest) setSelectedVersion(latest.id)
    } else {
      const latest = versions.find((v: VersionInfo) => v.type === 'old_alpha' || v.type === 'old_beta')
      if (latest) setSelectedVersion(latest.id)
    }
  }

  async function handleDownload() {
    if (!selectedVersion) { setStatus('请先选择一个版本'); return }
    setDownloading(true)
    setStatus(`正在下载版本 ${selectedVersion}...`)
    try {
      if (savesIsolation) {
        await tauriInvoke('prepare_saves_isolation', { version: selectedVersion })
      }
      const finalConcurrency = isCustomConcurrency ? parseInt(customConcurrency) || 32 : concurrency
      await tauriInvoke('download', { opts: { version: selectedVersion, game_dir: gameDir, concurrency: finalConcurrency } })
      setStatus(`版本 ${selectedVersion} 下载完成`)
    } catch (e) {
      setStatus(`下载失败: ${e}`)
    } finally {
      setDownloading(false)
    }
  }

  async function handleLaunch() {
    if (!selectedVersion) { setStatus('请先选择一个版本'); return }
    if (!selectedJava) { setStatus('请先选择一个 Java 版本'); return }
    if (!selectedProfile) { setStatus('请先选择或创建用户档案'); return }
    setLaunching(true)
    setGameRunning(true)
    setStatus(`正在启动 ${selectedVersion}...`)
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
      setStatus('游戏已退出')
    } catch (e) {
      setStatus(`启动失败: ${e}`)
    } finally {
      setLaunching(false)
      setGameRunning(false)
    }
  }

  async function handleKill() {
    try {
      await tauriInvoke('kill_game')
      setStatus('游戏已终止')
      setGameRunning(false)
    } catch (e) {
      setStatus(`终止失败: ${e}`)
    }
  }

  async function handleCreateProfile() {
    if (!newProfileName.trim()) { setStatus('请输入用户名'); return }
    try {
      const profile = await tauriInvoke<UserProfile>('create_profile', { name: newProfileName })
      setProfiles(prev => [...prev, profile])
      setSelectedProfile(profile)
      setNewProfileName('')
      setShowProfileModal(false)
      setStatus(`已创建档案: ${profile.name}`)
    } catch (e) {
      setStatus(`创建失败: ${e}`)
    }
  }

  async function handleDeleteProfile(name: string) {
    try {
      await tauriInvoke('delete_profile', { name })
      setProfiles(prev => prev.filter((p: UserProfile) => p.name !== name))
      if (selectedProfile?.name === name) setSelectedProfile(null)
      setStatus(`已删除档案: ${name}`)
    } catch (e) {
      setStatus(`删除失败: ${e}`)
    }
  }

  async function handleResetUuid() {
    if (!selectedProfile) return
    try {
      const updated = await tauriInvoke<UserProfile>('reset_profile_uuid', { name: selectedProfile.name })
      setProfiles(prev => prev.map((p: UserProfile) => p.name === updated.name ? updated : p))
      setSelectedProfile(updated)
      setStatus(`已重置 UUID: ${updated.uuid}`)
    } catch (e) {
      setStatus(`重置失败: ${e}`)
    }
  }

  async function handleDeleteUserConfig() {
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
  }

  async function handleResetAllConfig() {
    const msg = ['确定删除所有配置并重新初始化吗？', '', '将删除:', '  - config.json (所有配置)', '  - profiles.json (所有用户档案)', '', '游戏文件不会被删除', '此操作不可逆！'].join('\n')
    if (!await ask(msg, { title: '重置所有配置', kind: 'warning' })) return
    if (!await ask('再次确认：删除所有配置？', { title: '二次确认', kind: 'warning' })) return
    try {
      await tauriInvoke('reset_all_config')
      setProfiles([]); setSelectedProfile(null)
      setRootDir(''); setGameDir(''); setSavesDir(''); setVersionsDir('')
      setSelectedJava(''); setConcurrency(32); setIsCustomConcurrency(false); setCustomConcurrency('')
      await loadConfig()
      await loadProfiles()
      setStatus('所有配置已重置，正在重新初始化...')
    } catch (e) {
      setStatus(`重置失败: ${e}`)
    }
  }

  async function handleUiModeChange(mode: 'performance' | 'gorgeous') {
    setUiMode(mode)
    document.documentElement.setAttribute('data-ui-mode', mode)
    try {
      const config = await tauriInvoke<AppConfig>('get_config')
      await tauriInvoke('set_config', { config: { ...config, ui_mode: mode } })
    } catch (e) {
      console.error('保存 UI 模式失败:', e)
    }
  }

  async function handleClearGameFiles() {
    if (!gameDir) { setStatus('游戏目录未设置'); return }
    const msg = ['确定清空游戏目录吗？', '', `目录: ${gameDir}`, '', '将删除:', '  - versions/ (所有版本)', '  - libraries/ (所有依赖库)', '  - assets/ (所有资源文件)', '  - 其他所有游戏文件', '', '目录本身将保留', '此操作不可逆！'].join('\n')
    if (!await ask(msg, { title: '清空游戏文件', kind: 'warning' })) return
    if (!await ask('再次确认：清空所有游戏文件？', { title: '二次确认', kind: 'warning' })) return
    try {
      await tauriInvoke('clear_game_files', { gameDir })
      setStatus('游戏文件已清空')
    } catch (e) {
      setStatus(`清空失败: ${e}`)
    }
  }

  async function handleSetupComplete() {
    const finalConcurrency = isCustomConcurrency ? parseInt(customConcurrency) || 32 : concurrency
    const config: AppConfig = {
      version: 2,
      root_dir: setupRootDir || rootDir,
      version_isolation: setupVersionIsolation,
      saves_isolation: false,
      last_profile: null,
      java_path: setupJava || selectedJava,
      download_concurrency: finalConcurrency,
      first_launch: false,
      ui_mode: uiMode
    }
    await tauriInvoke('set_config', { config })
    if (setupRootDir) {
      setRootDir(setupRootDir)
      updateDirsFromRoot(setupRootDir, setupVersionIsolation)
      setVersionIsolation(setupVersionIsolation)
    }
    if (setupJava) setSelectedJava(setupJava)
    setShowSetup(false)
    setStatus('配置完成')
  }

  async function handleOpenMicrosoftAuth() {
    try {
      setStatus('正在通过微软登录...')
      setLoginMessage('正在打开登录窗口，请在弹出的窗口中完成 Microsoft 登录')
      const profile = await tauriInvoke<UserProfile>('login_microsoft')
      setProfiles(prev => {
        const idx = prev.findIndex(p => p.name === profile.name)
        if (idx >= 0) {
          const copy = [...prev]
          copy[idx] = profile
          return copy
        }
        return [...prev, profile]
      })
      setSelectedProfile(profile)
      setMicrosoftCodeInput('')
      setLoginMessage('登录成功')
      setStatus('登录成功')
    } catch (e) {
      setStatus(`登录失败: ${e}`)
      setLoginMessage('请尝试使用手动粘贴方式登录')
    }
  }

  async function handleMicrosoftWithCode() {
    if (!microsoftCodeInput.trim()) { setStatus('请粘贴回调 URL'); return }
    setIsLoggingIn(true)
    setLoginMessage('正在完成登录...')
    try {
      const url = microsoftCodeInput.trim()
      const codeMatch = url.match(/[?&]code=([^&]+)/)
      if (!codeMatch) throw new Error('未在 URL 中找到 code 参数')
      const code = codeMatch[1]
      const profile = await tauriInvoke<UserProfile>('login_microsoft_with_code', { code })
      setProfiles(prev => {
        const exists = prev.find((p: UserProfile) => p.uuid === profile.uuid)
        if (exists) return prev.map((p: UserProfile) => p.uuid === profile.uuid ? profile : p)
        return [...prev, profile]
      })
      setSelectedProfile(profile)
      setMicrosoftCodeInput('')
      setShowProfileModal(false)
      setStatus(`登录成功: ${profile.name}`)
    } catch (e) {
      setStatus(`登录失败: ${e}`)
    } finally {
      setIsLoggingIn(false)
    }
  }

  async function handleSavePaths() {
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
  }

  async function scanJavaDir(dir: string) {
    setStatus(`正在扫描: ${dir}`)
    const javas = await tauriInvoke<JavaInfo[]>('scan_java_dir', { dir })
    if (javas.length > 0) {
      setJavaList(prev => {
        const filtered = prev.filter(j => !javas.some((n: JavaInfo) => n.path === j.path))
        return [...javas, ...filtered]
      })
      setSelectedJava(javas[0].path)
      setStatus(`找到 ${javas.length} 个 Java 运行时`)
    } else {
      setStatus('未在该目录找到 Java 运行时')
    }
  }

  function getFilteredVersions() {
    return versions.filter((v: VersionInfo) => {
      if (versionType === 'release') return v.type === 'release'
      if (versionType === 'snapshot') return v.type === 'snapshot'
      if (versionType === 'old') return v.type === 'old_alpha' || v.type === 'old_beta'
      return true
    })
  }

  const value: AppContextType = {
    // State
    versions, selectedVersion, versionType,
    javaList, selectedJava,
    rootDir, gameDir, savesDir, versionsDir, versionIsolation, savesIsolation,
    launching, gameRunning, status,
    profiles, selectedProfile,
    concurrency, customConcurrency, isCustomConcurrency,
    uiMode, showSetup, setupStep, setupRootDir, setupVersionIsolation, setupJava,
    showProfileModal, newProfileName, isLoggingIn, loginMessage, microsoftCodeInput,
    downloadProgress, downloading,
    // Actions
    loadVersions, loadJava, loadProfiles, loadConfig,
    setSelectedVersion, setVersionType, setSelectedJava, setRootDir,
    setVersionIsolation, setSavesIsolation,
    handleDownload, handleLaunch, handleKill,
    handleCreateProfile, handleDeleteProfile, handleResetUuid,
    handleDeleteUserConfig, handleResetAllConfig,
    handleUiModeChange, handleClearGameFiles, handleSetupComplete,
    handleOpenMicrosoftAuth, handleMicrosoftWithCode,
    setShowProfileModal, setNewProfileName, setMicrosoftCodeInput,
    setSetupStep, setSetupRootDir, setSetupVersionIsolation, setSetupJava,
    selectRootDir, handleSavePaths, handleVersionTabChange, scanJavaDir, setDownloading, getFilteredVersions,
    setStatus, setShowSetup, setSelectedProfile, setConcurrency, setCustomConcurrency, setIsCustomConcurrency,
  }

  return <AppContext.Provider value={value}>{children}</AppContext.Provider>
}

export function useAppState() {
  const ctx = useContext(AppContext)
  if (!ctx) throw new Error('useAppState must be used within AppProvider')
  return ctx
}
