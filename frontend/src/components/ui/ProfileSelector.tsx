import { motion } from 'framer-motion'

interface Profile {
  name: string
  uuid: string
  auth_method: 'Offline' | 'Microsoft'
}

interface ProfileSelectorProps {
  profiles: Profile[]
  selectedProfile: Profile | null
  onSelect: (profile: Profile) => void
}

export function ProfileSelector({
  profiles,
  selectedProfile,
  onSelect,
}: ProfileSelectorProps) {
  return (
    <div className="card" style={{ padding: '20px' }}>
      <h3 style={{
        fontFamily: 'var(--font-display)',
        fontSize: '0.8rem',
        color: 'var(--text-muted)',
        textTransform: 'uppercase',
        letterSpacing: '0.1em',
        marginBottom: '16px',
      }}>
        选择档案
      </h3>

      <div style={{ display: 'flex', flexDirection: 'column', gap: '8px' }}>
        {profiles.map((profile) => (
          <motion.button
            key={profile.name}
            onClick={() => onSelect(profile)}
            whileHover={{ scale: 1.01 }}
            whileTap={{ scale: 0.99 }}
            style={{
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'space-between',
              padding: '14px 16px',
              background: selectedProfile?.name === profile.name
                ? 'var(--accent-light)'
                : 'var(--bg-card)',
              border: `1px solid ${
                selectedProfile?.name === profile.name
                  ? 'var(--accent-primary)'
                  : 'var(--border-subtle)'
              }`,
              borderRadius: 'var(--radius-md)',
              cursor: 'pointer',
              transition: 'all 0.2s ease',
            }}
          >
            <div style={{ textAlign: 'left' }}>
              <p style={{ color: 'var(--text-primary)', fontWeight: 500 }}>
                {profile.name}
              </p>
              <p style={{ color: 'var(--text-muted)', fontSize: '0.75rem', fontFamily: 'var(--font-mono)' }}>
                {profile.uuid}
              </p>
            </div>
            <span style={{
              fontSize: '0.65rem',
              padding: '4px 8px',
              borderRadius: '9999px',
              background: profile.auth_method === 'Microsoft'
                ? 'var(--accent-light)'
                : 'var(--border-default)',
              color: profile.auth_method === 'Microsoft'
                ? 'var(--accent-primary)'
                : 'var(--text-muted)',
            }}>
              {profile.auth_method}
            </span>
          </motion.button>
        ))}
      </div>
    </div>
  )
}
