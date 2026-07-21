export interface VersionInfo {
  id: string
  type: string
  url: string
  releaseTime: string
}

export interface JavaInfo {
  path: string
  version: string
}

export interface UserProfile {
  name: string
  uuid: string
  auth_method: 'Offline' | 'Microsoft'
  access_token?: string
  refresh_token?: string
  expires_at?: number
}

export interface ProfileStore {
  profiles: UserProfile[]
  last_used: string | null
}

export interface DownloadProgress {
  current: number
  total: number
  percent: number
  current_file: string
}

export interface AppConfig {
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

export type AuthMethod = 'Offline' | 'Microsoft'

export interface ModInfo {
  id: string
  version: string
  entry: string
  file_name: string
  enabled: boolean
}
