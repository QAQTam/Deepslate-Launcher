import { useState, useEffect } from 'react'
import { Sidebar } from './components/layout/Sidebar'
import { HomePage } from './components/pages/HomePage'
import { LaunchPage } from './components/pages/LaunchPage'
import { SettingsPage } from './components/pages/SettingsPage'
import { NewsPage } from './components/pages/NewsPage'
import { AppProvider } from './contexts/AppProvider'

function AppContent() {
  const [activePage, setActivePage] = useState('home')
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false)

  useEffect(() => {
    const saved = localStorage.getItem('sidebar-collapsed')
    if (saved) setSidebarCollapsed(JSON.parse(saved))
  }, [])

  const sidebarWidth = sidebarCollapsed ? 64 : 220

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
      default:
        return <HomePage />
    }
  }

  return (
    <div style={{
      display: 'flex',
      minHeight: '100vh',
      background: 'var(--bg-deep)',
    }}>
      <Sidebar
        activePage={activePage}
        onNavigate={setActivePage}
        collapsed={sidebarCollapsed}
        onToggleCollapse={() => setSidebarCollapsed(!sidebarCollapsed)}
      />
      <main style={{
        marginLeft: `${sidebarWidth}px`,
        flex: 1,
        minHeight: '100vh',
        transition: 'margin-left 0.3s ease',
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