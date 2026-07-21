import type { ReactNode } from 'react'
import { AppProvider as StateProvider } from '../hooks/useAppState'
import { ConfigProvider } from './ConfigContext'
import { GameProvider } from './GameContext'
import { DownloadProvider } from './DownloadContext'
import { VersionProvider } from './VersionContext'
import { ProfileProvider } from './ProfileContext'

export function AppProvider({ children }: { children: ReactNode }) {
  return (
    <StateProvider>
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
    </StateProvider>
  )
}
