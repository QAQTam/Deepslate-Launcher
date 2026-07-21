import { useState, useEffect } from 'react'
import { Sidebar } from './components/layout/Sidebar'
import { HomePage } from './components/pages/HomePage'
import { LaunchPage } from './components/pages/LaunchPage'
import { SettingsPage } from './components/pages/SettingsPage'
import { NewsPage } from './components/pages/NewsPage'
import { ModsPage } from './components/pages/ModsPage'
import { AppProvider } from './contexts/AppProvider'

function AppContent() {
  const [activePage, setActivePage] = useState('home')
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false)

  useEffect(() => {
    const saved = localStorage.getItem('sidebar-collapsed')
    if (saved) setSidebarCollapsed(JSON.parse(saved))
  }, [])

  const renderPage = () => {
    switch (activePage) {
      case 'home':
        return <HomePage />
      case 'launch':
        return <LaunchPage />
      case 'settings':
        return <SettingsPage />
      case 'news':
        return <NewsPage />
      case 'mods':
        return <ModsPage />
      default:
        return <HomePage />
    }
  }

  return (
    <div style={{
      display: 'flex',
      height: '100vh',
      overflow: 'hidden',
      background: 'var(--bg-page)',
    }}>
      <Sidebar
        activePage={activePage}
        onNavigate={setActivePage}
        collapsed={sidebarCollapsed}
        onToggleCollapse={() => setSidebarCollapsed(!sidebarCollapsed)}
      />
      <main style={{
        flex: 1,
        overflowY: 'auto',
        height: '100vh',
      }}>
        {renderPage()}
      </main>
    </div>
  )
}

export default function App() {
  return (
    <AppProvider>
      <AppContent />
    </AppProvider>
  )
}