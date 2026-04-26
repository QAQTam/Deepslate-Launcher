import { createContext, useContext, useState, useCallback, type ReactNode } from 'react'
import { tauriInvoke } from '../../hooks/useTauriInvoke'

interface UserProfile {
  name: string
  uuid: string
  auth_method: 'Offline' | 'Microsoft'
  access_token?: string
}

interface ProfileContextType {
  profiles: UserProfile[]
  selectedProfile: UserProfile | null
  loading: boolean
  loadProfiles: () => Promise<void>
  selectProfile: (profile: UserProfile) => void
  createProfile: (name: string) => Promise<void>
  deleteProfile: (name: string) => Promise<void>
}

const ProfileContext = createContext<ProfileContextType | null>(null)

export function ProfileProvider({ children }: { children: ReactNode }) {
  const [profiles, setProfiles] = useState<UserProfile[]>([])
  const [selectedProfile, setSelectedProfile] = useState<UserProfile | null>(null)
  const [loading, setLoading] = useState(false)

  const loadProfiles = useCallback(async () => {
    setLoading(true)
    try {
      const store = await tauriInvoke<{ profiles: UserProfile[]; last_used: string | null }>('get_profiles')
      setProfiles(store.profiles)
      if (store.last_used) {
        const last = store.profiles.find((p: UserProfile) => p.name === store.last_used)
        if (last) setSelectedProfile(last)
      }
    } catch (e) {
      console.error('Failed to load profiles:', e)
    } finally {
      setLoading(false)
    }
  }, [])

  const selectProfile = useCallback((profile: UserProfile) => {
    setSelectedProfile(profile)
  }, [])

  const createProfile = useCallback(async (name: string) => {
    const profile = await tauriInvoke<UserProfile>('create_profile', { name })
    setProfiles((prev) => [...prev, profile])
    setSelectedProfile(profile)
  }, [])

  const deleteProfile = useCallback(async (name: string) => {
    await tauriInvoke('delete_profile', { name })
    setProfiles((prev) => prev.filter((p) => p.name !== name))
    setSelectedProfile((prev) => (prev?.name === name ? null : prev))
  }, [])

  return (
    <ProfileContext.Provider
      value={{
        profiles,
        selectedProfile,
        loading,
        loadProfiles,
        selectProfile,
        createProfile,
        deleteProfile,
      }}
    >
      {children}
    </ProfileContext.Provider>
  )
}

export function useProfile() {
  const ctx = useContext(ProfileContext)
  if (!ctx) throw new Error('useProfile must be used within ProfileProvider')
  return ctx
}