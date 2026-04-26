import { AppProvider } from './hooks/useAppState'
import { Header } from './components/Header'
import { ProfileSection } from './components/ProfileSection'
import { VersionSection } from './components/VersionSection'
import { DownloadSection } from './components/DownloadSection'
import { DirectorySection } from './components/DirectorySection'
import { JavaSection } from './components/JavaSection'
import { LaunchSection } from './components/LaunchSection'
import { ConfigSection } from './components/ConfigSection'
import { StatusBar } from './components/StatusBar'
import { SetupModal } from './modals/SetupModal'
import { ProfileModal } from './modals/ProfileModal'

function App() {
  return (
    <AppProvider>
      <div className="app">
        <Header />
        <main className="main">
          <ProfileSection />
          <VersionSection />
          <DownloadSection />
          <DirectorySection />
          <JavaSection />
          <LaunchSection />
          <ConfigSection />
          <StatusBar />
        </main>
        <SetupModal />
        <ProfileModal />
      </div>
    </AppProvider>
  )
}

export default App
