import { useState, useEffect, useCallback, useRef } from 'react'
import { motion, AnimatePresence } from 'framer-motion'
import { PageContainer } from '../layout/PageContainer'
import { fetchMinecraftNews, fetchArticleDetail, type NewsItem, type NewsResponse, type ArticleDetail } from '../../services/newsApi'
import { tauriInvoke } from '../../hooks/useTauriInvoke'

function openInBrowser(url: string) {
  tauriInvoke('open_url', { url }).catch(() => {})
}

function renderMarkdown(text: string): React.ReactNode[] {
  if (!text) return []
  return text.split('\n').filter(b => b.trim()).map((block, i) => {
    const t = block.trim()
    if (t.startsWith('## ')) {
      return (
        <h2 key={i} style={{
          fontFamily: 'var(--font-display)', fontSize: '1.15rem', fontWeight: 700,
          color: 'var(--text-primary)', margin: i > 0 ? '24px 0 8px' : '0 0 8px',
          lineHeight: 1.4,
        }}>
          {t.slice(3)}
        </h2>
      )
    }
    if (t.startsWith('### ')) {
      return (
        <h3 key={i} style={{
          fontFamily: 'var(--font-display)', fontSize: '1.05rem', fontWeight: 600,
          color: 'var(--text-primary)', margin: i > 0 ? '20px 0 6px' : '0 0 6px',
          lineHeight: 1.4,
        }}>
          {t.slice(4)}
        </h3>
      )
    }
    if (t.startsWith('- ')) {
      return (
        <li key={i} style={{
          fontSize: '0.95rem', color: 'var(--text-secondary)',
          lineHeight: 1.8, marginLeft: '20px', listStyle: 'disc',
        }}>
          {t.slice(2)}
        </li>
      )
    }
    return (
      <p key={i} style={{
        fontSize: '0.95rem', color: 'var(--text-secondary)',
        lineHeight: 1.8, margin: i > 0 ? '12px 0 0' : '0',
      }}>
        {block}
      </p>
    )
  })
}

export function NewsPage() {
  const [news, setNews] = useState<NewsItem[]>([])
  const [loading, setLoading] = useState(true)
  const [loadingMore, setLoadingMore] = useState(false)
  const [error, setError] = useState('')
  const [currentPage, setCurrentPage] = useState(1)
  const [totalCount, setTotalCount] = useState(0)
  const [hasMore, setHasMore] = useState(false)

  // Modal state
  const [selectedItem, setSelectedItem] = useState<NewsItem | null>(null)
  const [detail, setDetail] = useState<ArticleDetail | null>(null)
  const [carouselIdx, setCarouselIdx] = useState(0)
  const carouselTimer = useRef<ReturnType<typeof setInterval> | null>(null)

  // Preload cache
  const detailCache = useRef<Map<string, ArticleDetail>>(new Map())
  const preloading = useRef<Set<string>>(new Set())
  const hoverTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const sentinelRef = useRef<HTMLDivElement | null>(null)

  const loadPage = useCallback(async (page: number, append: boolean) => {
    if (append) setLoadingMore(true)
    else setLoading(true)
    try {
      const resp: NewsResponse = await fetchMinecraftNews(page)
      const items = resp.items
      if (append) { setNews(prev => [...prev, ...items]) }
      else { setNews(items) }
      setCurrentPage(resp.page)
      setTotalCount(resp.total)
      setHasMore(items.length >= 24 && items.length * resp.page < resp.total)
      setError('')
      // Preload first few visible cards
      if (!append) {
        setTimeout(() => preloadArticles(items.slice(0, 4)), 300)
      }
    } catch (e) {
      if (!append) setError(`无法加载新闻: ${e}`)
    } finally {
      setLoading(false)
      setLoadingMore(false)
    }
  }, [])

  useEffect(() => { loadPage(1, false) }, [loadPage])

  // Auto-load more on scroll (IntersectionObserver on sentinel)
  useEffect(() => {
    if (!hasMore || loadingMore) return
    const el = sentinelRef.current
    if (!el) return
    const observer = new IntersectionObserver((entries) => {
      if (entries[0]?.isIntersecting && hasMore && !loadingMore) {
        loadPage(currentPage + 1, true)
      }
    }, { rootMargin: '200px' })
    observer.observe(el)
    return () => observer.disconnect()
  }, [hasMore, loadingMore, currentPage, loadPage])

  // Preload article detail (skip if already cached or in progress)
  const preloadArticle = useCallback(async (item: NewsItem) => {
    if (detailCache.current.has(item.url) || preloading.current.has(item.url)) return
    preloading.current.add(item.url)
    try {
      const d = await fetchArticleDetail(item.url)
      detailCache.current.set(item.url, d)
    } catch { /* ignore preload errors */ }
    finally { preloading.current.delete(item.url) }
  }, [])

  const preloadArticles = useCallback((items: NewsItem[]) => {
    items.forEach(item => preloadArticle(item))
  }, [preloadArticle])

  const handleCardHover = useCallback((item: NewsItem) => {
    if (hoverTimer.current) clearTimeout(hoverTimer.current)
    hoverTimer.current = setTimeout(() => preloadArticle(item), 150)
  }, [preloadArticle])

  const handleCardClick = useCallback(async (item: NewsItem) => {
    if (selectedItem?.url === item.url) {
      setSelectedItem(null); setDetail(null); return
    }
    setSelectedItem(item)
    // Use cache if available
    const cached = detailCache.current.get(item.url)
    if (cached) {
      setDetail(cached)
    } else {
      setDetail(null)
      fetchArticleDetail(item.url)
        .then(d => { detailCache.current.set(item.url, d); setDetail(d) })
        .catch(() => setDetail(null))
    }
  }, [selectedItem])

  const handleClose = () => {
    setSelectedItem(null)
    setDetail(null)
  }

  // Primary data from card; detail adds author + full article body + images
  const displayTitle = selectedItem?.title || ''
  const displayDescription = detail?.description || selectedItem?.description || ''
  const displayDate = selectedItem?.date || ''
  const displayAuthor = detail?.author || ''
  const displayUrl = selectedItem?.url || ''
  const modalImage = selectedItem?.image_url
  // Combine hero + article images for carousel
  const carouselImages: string[] = detail?.images?.length
    ? detail.images
    : modalImage ? [modalImage] : []

  // Auto-scroll carousel
  useEffect(() => {
    if (carouselImages.length <= 1) return
    carouselTimer.current = setInterval(() => {
      setCarouselIdx(prev => (prev + 1) % carouselImages.length)
    }, 4000)
    return () => { if (carouselTimer.current) clearInterval(carouselTimer.current) }
  }, [carouselImages.length, selectedItem?.url])

  // Reset carousel on new article
  useEffect(() => { setCarouselIdx(0) }, [selectedItem?.url])

  return (
    <PageContainer title="📰 新闻">
      {loading && (
        <div style={{ padding: 'var(--space-xl)', textAlign: 'center', color: 'var(--text-secondary)' }}>
          正在加载 Minecraft 官方新闻...
        </div>
      )}

      {error && (
        <div className="card" style={{ padding: 'var(--space-xl)', textAlign: 'center' }}>
          <p style={{ color: 'var(--danger)', marginBottom: 'var(--space-md)' }}>{error}</p>
          <button className="btn-primary" onClick={() => loadPage(1, false)}>重试</button>
        </div>
      )}

      {!loading && !error && (
        <>
          {/* Card grid */}
          <div style={{
            display: 'grid',
            gridTemplateColumns: 'repeat(auto-fill, minmax(300px, 1fr))',
            gap: 'var(--space-md)',
          }}>
            {news.map((item, i) => (
              <motion.div
                key={item.url}
                initial={{ opacity: 0, y: 12 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ delay: (i % 24) * 0.015, duration: 0.2, ease: 'easeOut' }}
                onClick={() => handleCardClick(item)}
                onMouseEnter={() => handleCardHover(item)}
                className="card"
                style={{
                  cursor: 'pointer', overflow: 'hidden', padding: 0,
                  display: 'flex', flexDirection: 'column',
                  transition: 'transform 0.12s ease, box-shadow 0.12s ease',
                }}
                whileHover={{ y: -2, boxShadow: 'var(--shadow-md)' }}
              >
                {item.image_url && (
                  <div style={{ width: '100%', height: '180px', overflow: 'hidden', background: 'var(--bg-input)', flexShrink: 0 }}>
                    <img src={item.image_url} alt={item.title} style={{ width: '100%', height: '100%', objectFit: 'cover' }}
                      loading="lazy" onError={(e) => { (e.target as HTMLImageElement).style.display = 'none' }} />
                  </div>
                )}
                <div style={{ padding: 'var(--space-lg)', display: 'flex', flexDirection: 'column', gap: 'var(--space-sm)', flex: 1 }}>
                  <h3 style={{ fontFamily: 'var(--font-display)', fontSize: '1.05rem', fontWeight: 700, lineHeight: 1.4, margin: 0 }}>
                    {item.title}
                  </h3>
                  <p style={{ fontSize: '0.85rem', color: 'var(--text-secondary)', lineHeight: 1.5, margin: 0, flex: 1,
                    display: '-webkit-box', WebkitLineClamp: 3, WebkitBoxOrient: 'vertical', overflow: 'hidden',
                  }}>
                    {item.description}
                  </p>
                  {item.date && (
                    <span style={{ fontSize: '0.75rem', color: 'var(--text-muted)' }}>{item.date}</span>
                  )}
                </div>
              </motion.div>
            ))}
          </div>

          {/* Pagination */}
          {/* Sentinel for auto-load */}
          <div ref={sentinelRef} style={{ height: '1px' }} />

          {hasMore && (
            <div style={{ marginTop: 'var(--space-lg)', textAlign: 'center' }}>
              <button className="btn-secondary" onClick={() => loadPage(currentPage + 1, true)}
                disabled={loadingMore} style={{ padding: '10px 32px' }}>
                {loadingMore ? '加载中...' : `加载更多 (${news.length} / ${totalCount})`}
              </button>
            </div>
          )}
          {!hasMore && news.length > 0 && (
            <div style={{ marginTop: 'var(--space-lg)', textAlign: 'center' }}>
              <span style={{ fontSize: '0.85rem', color: 'var(--text-muted)' }}>已加载全部 {news.length} 篇新闻</span>
            </div>
          )}
        </>
      )}

      {/* Modal Overlay */}
      <AnimatePresence>
        {selectedItem && (
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.12 }}
            onClick={handleClose}
            style={{
              position: 'fixed', inset: 0, zIndex: 1000,
              background: 'rgba(0,0,0,0.5)', display: 'flex',
              alignItems: 'center', justifyContent: 'center',
              padding: 'var(--space-lg)',
            }}
          >
            <motion.div
              initial={{ opacity: 0, scale: 0.95, y: 20 }}
              animate={{ opacity: 1, scale: 1, y: 0 }}
              exit={{ opacity: 0, scale: 0.95, y: 20 }}
              transition={{ duration: 0.18, ease: [0.32, 0.72, 0, 1] }}
              onClick={(e) => e.stopPropagation()}
              style={{
                background: 'var(--bg-card)',
                borderRadius: 'var(--radius-xl)',
                boxShadow: 'var(--shadow-lg)',
                maxWidth: '680px',
                width: '100%',
                maxHeight: '85vh',
                overflow: 'hidden',
                display: 'flex',
                flexDirection: 'column',
              }}
            >
              {/* Close button */}
              <div style={{
                display: 'flex', justifyContent: 'space-between', alignItems: 'center',
                padding: 'var(--space-md) var(--space-lg)',
                borderBottom: '1px solid var(--border-subtle)',
              }}>
                <span style={{ fontSize: '0.75rem', color: 'var(--text-muted)', fontWeight: 500 }}>
                  文章详情
                </span>
                <button
                  onClick={handleClose}
                  className="btn-ghost"
                  style={{
                    width: '32px', height: '32px', borderRadius: '50%',
                    display: 'flex', alignItems: 'center', justifyContent: 'center',
                    fontSize: '1.1rem', padding: 0, lineHeight: 1,
                    color: 'var(--text-secondary)',
                  }}
                >
                  ✕
                </button>
              </div>

              {/* Image Carousel + Scrollable Content */}
              <div style={{ overflowY: 'auto', flex: 1 }}>
                {carouselImages.length > 0 && (
                <div style={{ position: 'relative', width: '100%', height: '280px', overflow: 'hidden', background: 'var(--bg-input)' }}>
                  {carouselImages.map((img, i) => (
                    <img
                      key={i}
                      src={img}
                      alt=""
                      style={{
                        position: 'absolute', inset: 0,
                        width: '100%', height: '100%', objectFit: 'cover',
                        opacity: carouselIdx === i ? 1 : 0,
                        transition: 'opacity 0.6s ease',
                      }}
                      onError={(e) => { (e.target as HTMLImageElement).style.display = 'none' }}
                    />
                  ))}
                  {/* Dot indicators */}
                  {carouselImages.length > 1 && (
                    <div style={{
                      position: 'absolute', bottom: '12px', left: '50%', transform: 'translateX(-50%)',
                      display: 'flex', gap: '6px',
                    }}>
                      {carouselImages.map((_, i) => (
                        <button
                          key={i}
                          onClick={(e) => { e.stopPropagation(); setCarouselIdx(i) }}
                          style={{
                            width: '8px', height: '8px', borderRadius: '50%',
                            border: 'none', cursor: 'pointer', padding: 0,
                            background: carouselIdx === i ? '#fff' : 'rgba(255,255,255,0.4)',
                            transition: 'background 0.3s',
                          }}
                        />
                      ))}
                    </div>
                  )}
                </div>
              )}

                <div style={{ padding: 'var(--space-xl)' }}>
                  <h2 style={{
                    fontFamily: 'var(--font-display)', fontSize: '1.4rem', fontWeight: 700,
                    lineHeight: 1.35, margin: '0 0 var(--space-md)',
                  }}>
                    {displayTitle}
                  </h2>

                  {/* Author & Date entry */}
                  {(displayAuthor || displayDate) && (
                    <div style={{
                      display: 'flex', alignItems: 'center', gap: 'var(--space-md)',
                      padding: 'var(--space-md) var(--space-lg)',
                      background: 'var(--bg-input)',
                      borderRadius: 'var(--radius-md)',
                      marginBottom: 'var(--space-xl)',
                    }}>
                      {/* Author avatar */}
                      <div style={{
                        width: '40px', height: '40px', borderRadius: '50%',
                        background: 'linear-gradient(135deg, var(--accent-primary), var(--accent-hover))',
                        display: 'flex', alignItems: 'center', justifyContent: 'center',
                        color: '#fff', fontSize: '0.9rem', fontWeight: 700,
                        flexShrink: 0,
                      }}>
                        {displayAuthor ? displayAuthor.charAt(0).toUpperCase() : 'M'}
                      </div>
                      {/* Author info */}
                      <div>
                        {displayAuthor && (
                          <div style={{ fontSize: '0.9rem', fontWeight: 600, color: 'var(--text-primary)' }}>
                            {displayAuthor}
                          </div>
                        )}
                        {displayDate && (
                          <div style={{ fontSize: '0.8rem', color: 'var(--text-muted)', marginTop: '2px' }}>
                            {displayDate}
                          </div>
                        )}
                      </div>
                    </div>
                  )}

                  <div style={{ marginBottom: 'var(--space-xl)' }}>
                    {renderMarkdown(displayDescription)}
                  </div>

                  <div style={{ marginTop: 'var(--space-lg)', paddingTop: 'var(--space-md)', borderTop: '1px solid var(--border-subtle)', display: 'flex', justifyContent: 'flex-end' }}>
                    <button className="btn-ghost btn-sm" onClick={() => openInBrowser(displayUrl)}
                      style={{ fontSize: '0.75rem' }}>
                      在浏览器中打开原文 ↗
                    </button>
                  </div>
                </div>
              </div>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Footer */}
      <div style={{
        marginTop: 'var(--space-lg)', paddingTop: 'var(--space-md)',
        borderTop: '1px solid var(--border-subtle)',
        display: 'flex', justifyContent: 'flex-end', gap: 'var(--space-sm)', alignItems: 'center',
      }}>
        <span style={{ fontSize: '0.75rem', color: 'var(--text-muted)' }}>新闻来源: Minecraft.net</span>
        <a className="btn-secondary btn-sm" style={{ textDecoration: 'none' }}
          onClick={(e) => { e.preventDefault(); openInBrowser('https://www.minecraft.net/zh-hans/article') }}>
          在浏览器中打开
        </a>
      </div>
    </PageContainer>
  )
}
