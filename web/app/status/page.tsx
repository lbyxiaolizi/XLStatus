import type { Metadata } from "next";
import { getApiBaseUrl } from "@/lib/api";
import StatusPageClient from "./StatusPageClient";

// Visitor-facing page: render per request so crawlers and the first paint get
// real data instead of an empty client shell. The payload is per-deployment
// mutable state, so it must never be baked into the build output.
export const dynamic = "force-dynamic";

// The backend is usually a sibling container; keep the server pass short so a
// hung upstream degrades to the previous client-only behaviour instead of
// blocking the response.
const STATUS_TIMEOUT_MS = 3000;
const METADATA_REVALIDATE_SECONDS = 30;

interface PublicStatusBody {
  servers?: unknown[];
  services?: unknown[];
  updated_at?: string;
  site?: { site_name?: string } | null;
  theme?: unknown;
}

// Responses arrive either bare or wrapped in a { success, data } envelope,
// mirroring normalizeEnvelope in lib/api.ts. Accept both without pulling the
// browser-oriented ApiClient (auth headers, redirects) into the server pass.
function unwrapStatusBody(body: unknown): PublicStatusBody | null {
  if (!body || typeof body !== "object") return null;
  const record = body as { data?: PublicStatusBody } & PublicStatusBody;
  if (record.data && typeof record.data === "object" && Array.isArray(record.data.servers)) {
    return record.data;
  }
  return Array.isArray(record.servers) ? record : null;
}

async function fetchPublicStatus(revalidateSeconds?: number): Promise<PublicStatusBody | null> {
  const init: RequestInit =
    revalidateSeconds === undefined
      ? { cache: "no-store", signal: AbortSignal.timeout(STATUS_TIMEOUT_MS) }
      : { next: { revalidate: revalidateSeconds }, signal: AbortSignal.timeout(STATUS_TIMEOUT_MS) };

  try {
    const response = await fetch(`${getApiBaseUrl()}/api/v1/public/status`, init);
    if (!response.ok) return null;
    return unwrapStatusBody(await response.json());
  } catch {
    // Backend down, unreachable from the render server, or slower than the
    // timeout: the client component still fetches on mount.
    return null;
  }
}

export async function generateMetadata(): Promise<Metadata> {
  const body = await fetchPublicStatus(METADATA_REVALIDATE_SECONDS);
  const siteName = body?.site?.site_name;
  return { title: (typeof siteName === "string" ? siteName.trim() : "") || "XLStatus" };
}

// Stamps the fetch time outside the component body so the render itself stays
// pure; the client uses it to decide whether the snapshot is fresh enough to
// skip the mount-time refetch.
async function fetchInitialStatus(): Promise<{ body: PublicStatusBody | null; fetchedAt: number }> {
  const body = await fetchPublicStatus();
  return { body, fetchedAt: Date.now() };
}

export default async function StatusPage() {
  const { body, fetchedAt } = await fetchInitialStatus();
  return <StatusPageClient initialData={body} initialFetchedAt={fetchedAt} />;
}
