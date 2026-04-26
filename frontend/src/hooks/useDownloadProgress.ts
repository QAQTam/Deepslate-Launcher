import { useState } from 'react'
import { useTauriEvent } from './useTauriEvent'
import type { DownloadProgress } from '../types'

export function useDownloadProgress() {
  const [downloading, setDownloading] = useState(false)
  const [progress, setProgress] = useState<DownloadProgress | null>(null)

  useTauriEvent<DownloadProgress>('download-progress', (payload) => {
    setProgress(payload)
    if (payload.percent === 100) {
      setDownloading(false)
    }
  })

  return { downloading, setDownloading, progress }
}
