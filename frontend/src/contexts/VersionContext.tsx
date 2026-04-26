import { createContext, useContext, useState, useCallback, type ReactNode } from 'react'
import { tauriInvoke } from '../hooks/useTauriInvoke'

interface VersionInfo {
  id: string
  type: 'release' | 'snapshot' | 'old_alpha' | 'old_beta'
  url: string
  releaseTime: string
}

interface VersionContextType {
  versions: VersionInfo[]
  selectedVersion: string
  versionType: 'release' | 'snapshot' | 'old'
  loading: boolean
  loadVersions: () => Promise<void>
  setSelectedVersion: (version: string) => void
  setVersionType: (type: 'release' | 'snapshot' | 'old') => void
  getFilteredVersions: () => VersionInfo[]
}

const VersionContext = createContext<VersionContextType | null>(null)

export function VersionProvider({ children }: { children: ReactNode }) {
  const [versions, setVersions] = useState<VersionInfo[]>([])
  const [selectedVersion, setSelectedVersion] = useState('')
  const [versionType, setVersionType] = useState<'release' | 'snapshot' | 'old'>('release')
  const [loading, setLoading] = useState(false)

  const loadVersions = useCallback(async () => {
    setLoading(true)
    try {
      const vers = await tauriInvoke<VersionInfo[]>('get_versions')
      setVersions(vers)
      const latest = vers.find((v: VersionInfo) => v.type === 'release')
      if (latest) setSelectedVersion(latest.id)
    } catch (e) {
      console.error('Failed to load versions:', e)
    } finally {
      setLoading(false)
    }
  }, [])

  const getFilteredVersions = useCallback(() => {
    return versions.filter((v) => {
      if (versionType === 'release') return v.type === 'release'
      if (versionType === 'snapshot') return v.type === 'snapshot'
      if (versionType === 'old') return v.type === 'old_alpha' || v.type === 'old_beta'
      return true
    })
  }, [versions, versionType])

  return (
    <VersionContext.Provider
      value={{
        versions,
        selectedVersion,
        versionType,
        loading,
        loadVersions,
        setSelectedVersion,
        setVersionType,
        getFilteredVersions,
      }}
    >
      {children}
    </VersionContext.Provider>
  )
}

export function useVersion() {
  const ctx = useContext(VersionContext)
  if (!ctx) throw new Error('useVersion must be used within VersionProvider')
  return ctx
}