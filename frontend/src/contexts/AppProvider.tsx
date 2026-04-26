import { type ReactNode } from 'react'
import { VersionProvider } from './VersionContext'
import { ProfileProvider } from './ProfileContext'

export function AppProvider({ children }: { children: ReactNode }) {
  return (
    <VersionProvider>
      <ProfileProvider>
        {children}
      </ProfileProvider>
    </VersionProvider>
  )
}