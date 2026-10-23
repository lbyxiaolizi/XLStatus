import type { NextConfig } from 'next'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { SECURITY_RESPONSE_HEADERS } from './app/lib/security.ts'

const nextConfig: NextConfig = {
  output: 'standalone',
  poweredByHeader: false,
  turbopack: {
    root: path.dirname(fileURLToPath(import.meta.url)),
  },
  async headers() {
    return [
      {
        source: '/(.*)',
        headers: [...SECURITY_RESPONSE_HEADERS],
      },
    ]
  },
}

export default nextConfig
