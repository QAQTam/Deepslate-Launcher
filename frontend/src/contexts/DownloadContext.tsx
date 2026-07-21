import { createContext, useContext, useState, useEffect, type ReactNode } from 'react'
import { listen } from '@tauri-apps/api/event'

interface DownloadProgress {
  current: number
  total: number
  percent: number
  current_file: string
}

interface DownloadContextType {
  downloading: boolean
  setDownloading: (v: boolean) => void
  downloadProgress: DownloadProgress | null
}

const DownloadContext = createContext<DownloadContextType | null>(null)

export function DownloadProvider({ children }: { children: ReactNode }) {
  const [downloading, setDownloading] = useState(false)
  const [progress, setProgress] = useState<DownloadProgress | null>(null)

  useEffect(() => {
    const unlisten = listen<DownloadProgress>('download-progress', (event) => {
      const payload = event.payload
      setProgress(payload)
      if (payload.percent === 100) {
        setDownloading(false)
      }
    })
    return () => { unlisten.then(fn => fn()) }
  }, [])

  return (
    <DownloadContext.Provider value={{ downloading, setDownloading, downloadProgress: progress }}>
      {children}
    </DownloadContext.Provider>
  )
}

export function useDownload() {
  const ctx = useContext(DownloadContext)
  if (!ctx) throw new Error('useDownload must be used within DownloadProvider')
  return ctx
}