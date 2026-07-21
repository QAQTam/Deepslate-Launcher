import { tauriInvoke } from '../hooks/useTauriInvoke'

export interface NewsItem {
  title: string
  description: string
  url: string
  date: string
  image_url?: string
  author?: string
}

export interface NewsResponse {
  items: NewsItem[]
  total: number
  page: number
}

export interface ArticleDetail {
  title: string
  description: string
  author: string
  date: string
  image_url?: string
  url: string
  images: string[]
}

export async function fetchMinecraftNews(page: number = 1): Promise<NewsResponse> {
  const resp = await tauriInvoke<NewsResponse>('fetch_minecraft_news', { page })
  return {
    ...resp,
    items: resp.items.map(item => ({
      ...item,
      date: item.date || '',
      description: item.description || '阅读更多 →',
    })),
  }
}

export async function fetchArticleDetail(url: string): Promise<ArticleDetail> {
  return tauriInvoke<ArticleDetail>('fetch_article_detail', { url })
}
