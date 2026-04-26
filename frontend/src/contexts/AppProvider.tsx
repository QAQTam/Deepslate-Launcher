import { type ReactNode } from 'react'
import { ConfigProvider } from './ConfigContext'
import { GameProvider } from './GameContext'
import { DownloadProvider } from './DownloadContext'
import { VersionProvider } from './VersionContext'
import { ProfileProvider } from './ProfileContext'

export function AppProvider({ children }: { children: ReactNode }) {
  return (
    <ConfigProvider>
      <GameProvider>
        <DownloadProvider>
          <VersionProvider>
            <ProfileProvider>
              {children}
            </ProfileProvider>
          </VersionProvider>
        </DownloadProvider>
      </GameProvider>
    </ConfigProvider>
  )
}