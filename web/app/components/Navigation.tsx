"use client";

import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import {
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { apiClient } from "@/lib/api";
import { type Locale, type Translations, type t as ZhTranslations } from "@/lib/i18n";
import { useI18n } from "@/lib/use-i18n";
import {
  clearStoredUser,
  compactId,
  inputClass,
  isAdmin,
  useBoldTheme,
  useDialogFocusTrap,
  useStoredUser,
} from "./M7Primitives";
import { useOptionalServerData } from "./ServerDataProvider";
import { applyDashboardTheme, clearDashboardThemeCache } from "@/app/lib/dashboard-theme";
import { useDashboardTheme } from "@/app/lib/use-dashboard-theme";

type NavItem = { name: string; href: string; adminOnly?: boolean | undefined };

function navigationFor(copy: typeof ZhTranslations): NavItem[] {
  return [
    { name: copy.nav.dashboard, href: "/dashboard" },
    { name: copy.nav.servers, href: "/servers" },
    { name: copy.nav.services, href: "/services" },
    { name: copy.nav.tasks, href: "/tasks" },
    { name: copy.nav.terminal, href: "/terminal" },
    { name: copy.nav.mcp, href: "/mcp", adminOnly: false },
    { name: copy.nav.alerts, href: "/alerts" },
    { name: copy.nav.notifications, href: "/notifications" },
    { name: copy.nav.nat, href: "/nat" },
    { name: copy.nav.ddns, href: "/ddns", adminOnly: true },
    { name: copy.nav.settings, href: "/settings" },
    { name: copy.nav.status, href: "/status" },
  ];
}

function publicNavigationFor(copy: Translations): NavItem[] {
  return [{ name: copy.nav.status, href: "/status" }];
}

interface CommandServer {
  id: string;
  name: string;
  status?: string | undefined;
  tags?: string[] | undefined;
  provider?: string | null | undefined;
  region?: string | null | undefined;
}

const EMPTY_COMMAND_SERVERS: CommandServer[] = [];

export default function Navigation() {
  const pathname = usePathname();
  const dashboardPath = isDashboardPath(pathname);
  const router = useRouter();
  const user = useStoredUser();
  const serverData = useOptionalServerData();
  const { locale, locales, setLocale, t: copy } = useI18n();
  const [theme, setTheme] = useBoldTheme();
  const [open, setOpen] = useState(false);
  const [moreOpen, setMoreOpen] = useState(false);
  const [visibleCount, setVisibleCount] = useState<number>(99);
  const [commandOpen, setCommandOpen] = useState(false);
  const [commandQuery, setCommandQuery] = useState("");
  const [commandServers, setCommandServers] = useState<CommandServer[]>([]);
  const [commandLoadingUserId, setCommandLoadingUserId] = useState("");
  const [commandLoadedUserId, setCommandLoadedUserId] = useState("");
  const [commandError, setCommandError] = useState<{ userId: string; message: string } | null>(null);
  const commandRequestUserIdRef = useRef("");
  const revalidateCommandServers = serverData?.revalidate;

  const navContainerRef = useRef<HTMLDivElement | null>(null);
  const measureContainerRef = useRef<HTMLDivElement | null>(null);
  const moreMenuRef = useRef<HTMLDivElement | null>(null);

  useDashboardTheme({ user, dashboardPath, theme });

  const handleLogout = async () => {
    await apiClient.logout();
    applyDashboardTheme(null);
    clearDashboardThemeCache();
    clearStoredUser();
    router.replace("/login");
  };

  const navigation = useMemo(() => navigationFor(copy), [copy]);
  const publicNavigation = useMemo(() => publicNavigationFor(copy), [copy]);
  const commandLoading = Boolean(
    user && (serverData ? serverData.loading : commandLoadingUserId === user.id),
  );
  const visibleCommandServers = user
    ? serverData?.servers ?? (commandLoadedUserId === user.id ? commandServers : EMPTY_COMMAND_SERVERS)
    : EMPTY_COMMAND_SERVERS;
  const fallbackCommandError =
    commandError && commandError.userId === user?.id ? commandError.message : null;
  const visibleCommandError = user ? serverData?.error ?? fallbackCommandError : null;
  const visibleNavigation = useMemo(
    () => (user ? navigation.filter((item) => !item.adminOnly || isAdmin(user)) : publicNavigation),
    [navigation, publicNavigation, user],
  );

  const recomputeOverflow = useCallback(() => {
    const container = navContainerRef.current;
    const measure = measureContainerRef.current;
    if (!container || !measure) return;

    const availableWidth = container.clientWidth;
    if (availableWidth <= 0) return;

    const children = Array.from(measure.children) as HTMLElement[];
    if (children.length === 0) {
      setVisibleCount(visibleNavigation.length);
      return;
    }

    const gap = 4;
    const itemWidths = children.map((el) => Math.ceil(el.getBoundingClientRect().width));
    const totalWidth =
      itemWidths.reduce((sum, w) => sum + w, 0) + Math.max(0, itemWidths.length - 1) * gap;

    if (totalWidth <= availableWidth) {
      setVisibleCount(visibleNavigation.length);
      return;
    }

    const moreButtonReserve = 92;
    const budget = Math.max(0, availableWidth - moreButtonReserve);
    let used = 0;
    let count = 0;
    for (let i = 0; i < itemWidths.length; i += 1) {
      const w = itemWidths[i] ?? 0;
      const nextUsed = count === 0 ? w : used + gap + w;
      if (nextUsed <= budget) {
        used = nextUsed;
        count += 1;
      } else {
        break;
      }
    }
    setVisibleCount(Math.max(1, count));
  }, [visibleNavigation.length]);

  useLayoutEffect(() => {
    recomputeOverflow();
  }, [recomputeOverflow, locale, visibleNavigation]);

  useEffect(() => {
    const container = navContainerRef.current;
    if (!container) return;
    const observer = new ResizeObserver(() => {
      recomputeOverflow();
    });
    observer.observe(container);
    window.addEventListener("resize", recomputeOverflow);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", recomputeOverflow);
    };
  }, [recomputeOverflow]);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setCommandOpen((value) => !value);
      }
      if (event.key === "Escape") {
        setCommandOpen(false);
        setOpen(false);
        setMoreOpen(false);
      }
    }

    function onPointerDown(event: MouseEvent) {
      if (
        moreMenuRef.current &&
        event.target instanceof Node &&
        !moreMenuRef.current.contains(event.target)
      ) {
        setMoreOpen(false);
      }
    }

    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("mousedown", onPointerDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("mousedown", onPointerDown);
    };
  }, []);

  useEffect(() => {
    if (!commandOpen || !user) return;
    if (revalidateCommandServers) {
      void revalidateCommandServers();
      return;
    }
    if (commandRequestUserIdRef.current === user.id) return;
    const requestUserId = user.id;
    const controller = new AbortController();
    commandRequestUserIdRef.current = requestUserId;
    setCommandLoadingUserId(requestUserId);
    setCommandError(null);
    apiClient.listServers(200, 0, false, {
      signal: controller.signal,
      bypassClientCache: true,
    }).then((response) => {
      if (controller.signal.aborted) return;
      if (commandRequestUserIdRef.current === requestUserId) commandRequestUserIdRef.current = "";
      setCommandLoadingUserId((current) => (current === requestUserId ? "" : current));
      if (response.success && response.data) {
        const responseServers = Array.isArray(response.data.servers) ? response.data.servers : [];
        setCommandLoadedUserId(requestUserId);
        setCommandServers(
          (responseServers as CommandServer[]).filter((server) => server.id && server.name),
        );
      } else {
        setCommandError({
          userId: requestUserId,
          message: response.error || copy.common.requestFailed,
        });
      }
    });
    return () => {
      controller.abort();
      if (commandRequestUserIdRef.current === requestUserId) commandRequestUserIdRef.current = "";
    };
  }, [commandOpen, copy.common.requestFailed, revalidateCommandServers, user]);

  const commandItems = useMemo(
    () => buildCommandItems(visibleNavigation, visibleCommandServers, commandQuery, copy, locale),
    [commandQuery, copy, locale, visibleCommandServers, visibleNavigation],
  );

  function closeCommand() {
    setCommandOpen(false);
    setCommandQuery("");
  }

  function runCommand(href: string) {
    if (href === "theme:light" || href === "theme:dark") {
      setTheme(href === "theme:dark" ? "dark" : "light");
      closeCommand();
      return;
    }
    if (href === "locale:zh-CN" || href === "locale:en-US") {
      setLocale(href === "locale:en-US" ? "en-US" : "zh-CN");
      closeCommand();
      return;
    }
    closeCommand();
    setOpen(false);
    setMoreOpen(false);
    router.push(href);
  }

  const primaryItems = visibleNavigation.slice(0, visibleCount);
  const overflowItems = visibleNavigation.slice(visibleCount);
  const activeOverflowItem = overflowItems.find((item) => isActive(pathname, item.href));
  const moreLabel = locale === "zh-CN" ? "更多" : "More";
  const activeBadgeLabel = locale === "zh-CN" ? "当前" : "Active";

  return (
    <>
      <nav className="sticky top-0 z-40 border-b border-[var(--border-color)] bg-[var(--bg-card)]/85 backdrop-blur-md">
      <div className="mx-auto max-w-[96rem] px-4 sm:px-6 lg:px-8">
        <div className="flex min-h-14 items-center justify-between gap-3 py-2">
          {/* Brand Logo */}
          <Link href="/status" className="group flex shrink-0 items-center gap-2.5">
            <span className="grid h-8 w-8 place-items-center rounded-lg bg-[var(--accent-color)] text-xs font-bold tracking-tight text-white shadow-[var(--shadow-xs)] transition group-hover:opacity-90">
              XL
            </span>
            <span className="text-base font-semibold tracking-tight text-[var(--text-main)]">
              XLStatus
            </span>
          </Link>

          {/* Priority+ Adaptive Horizontal Navigation (Visible on all PC / Tablet screens >= 640px) */}
          <div
            ref={navContainerRef}
            className="relative hidden min-w-0 flex-1 items-center justify-center px-2 sm:flex"
          >
            {/* Hidden offscreen measurement bar */}
            <div
              ref={measureContainerRef}
              aria-hidden="true"
              className="pointer-events-none invisible absolute top-0 left-0 flex items-center gap-1 whitespace-nowrap"
            >
              {visibleNavigation.map((item) => (
                <span
                  key={item.href}
                  className="inline-flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-xs font-medium"
                >
                  <NavIcon href={item.href} />
                  <span>{item.name}</span>
                </span>
              ))}
            </div>

            <div className="flex items-center gap-1 whitespace-nowrap">
              {primaryItems.map((item) => (
                <NavLink key={item.href} href={item.href} active={isActive(pathname, item.href)}>
                  <NavIcon href={item.href} />
                  <span>{item.name}</span>
                </NavLink>
              ))}

              {overflowItems.length > 0 ? (
                <div ref={moreMenuRef} className="relative">
                  <button
                    type="button"
                    data-testid="nav-overflow-trigger"
                    onClick={() => setMoreOpen((prev) => !prev)}
                    aria-expanded={moreOpen}
                    aria-haspopup="menu"
                    className={`relative inline-flex items-center gap-1.5 rounded-lg border px-2.5 py-1.5 text-xs font-medium transition ${
                      activeOverflowItem || moreOpen
                        ? "border-[var(--border-strong)] bg-[var(--bg-muted)] font-semibold text-[var(--text-main)]"
                        : "border-transparent text-[var(--text-muted)] hover:border-[var(--border-color)] hover:bg-[var(--bg-subtle)] hover:text-[var(--text-main)]"
                    }`}
                  >
                    {activeOverflowItem ? <NavIcon href={activeOverflowItem.href} /> : null}
                    <span>{activeOverflowItem ? activeOverflowItem.name : moreLabel}</span>
                    <span className="rounded-full border border-[var(--border-color)] bg-[var(--bg-card)] px-1.5 py-0.2 font-mono text-[10px] font-semibold text-[var(--text-muted)]">
                      +{overflowItems.length}
                    </span>
                    <svg
                      viewBox="0 0 16 16"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="1.75"
                      className={`h-3.5 w-3.5 transition-transform duration-150 ${
                        moreOpen ? "rotate-180" : ""
                      }`}
                      aria-hidden="true"
                    >
                      <path d="M4 6l4 4 4-4" strokeLinecap="round" strokeLinejoin="round" />
                    </svg>
                    {activeOverflowItem ? (
                      <span
                        aria-hidden="true"
                        className="pointer-events-none absolute inset-x-2 -bottom-0.5 h-0.5 rounded-full bg-[var(--accent-color)]"
                      />
                    ) : null}
                  </button>

                  {moreOpen ? (
                    <div
                      role="menu"
                      className="absolute right-0 mt-2 w-60 origin-top-right rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] p-1.5 shadow-[var(--shadow-lg)] ring-1 ring-black/5"
                    >
                      <div className="mb-1 flex items-center justify-between border-b border-[var(--border-color)] px-2.5 py-1.5">
                        <span className="text-[10px] font-semibold tracking-wider text-[var(--text-faint)] uppercase">
                          {moreLabel}
                        </span>
                        <span className="rounded-full bg-[var(--bg-muted)] px-1.5 py-0.5 font-mono text-[10px] font-medium text-[var(--text-muted)]">
                          {overflowItems.length}
                        </span>
                      </div>
                      <div className="grid gap-0.5">
                        {overflowItems.map((item) => {
                          const active = isActive(pathname, item.href);
                          return (
                            <Link
                              key={item.href}
                              href={item.href}
                              role="menuitem"
                              onClick={() => setMoreOpen(false)}
                              className={`flex items-center justify-between gap-2 rounded-lg px-2.5 py-2 text-xs font-medium transition ${
                                active
                                  ? "bg-[var(--bg-muted)] font-semibold text-[var(--text-main)]"
                                  : "text-[var(--text-muted)] hover:bg-[var(--bg-subtle)] hover:text-[var(--text-main)]"
                              }`}
                            >
                              <span className="flex min-w-0 items-center gap-2">
                                <NavIcon href={item.href} />
                                <span className="truncate">{item.name}</span>
                              </span>
                              <span className="flex shrink-0 items-center gap-1.5">
                                {active ? (
                                  <span className="inline-flex items-center gap-1 rounded-full bg-[var(--accent-color)]/15 px-1.5 py-0.5 text-[10px] font-semibold text-[var(--accent-color)]">
                                    <span className="h-1.5 w-1.5 rounded-full bg-[var(--accent-color)]" />
                                    {activeBadgeLabel}
                                  </span>
                                ) : (
                                  <kbd className="rounded border border-[var(--border-color)] bg-[var(--bg-subtle)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--text-faint)]">
                                    {item.href}
                                  </kbd>
                                )}
                              </span>
                            </Link>
                          );
                        })}
                      </div>
                      <div className="mt-1 border-t border-[var(--border-color)] pt-1">
                        <button
                          type="button"
                          onClick={() => {
                            setMoreOpen(false);
                            setCommandOpen(true);
                          }}
                          className="flex w-full items-center justify-between rounded-lg px-2.5 py-1.5 text-[11px] font-medium text-[var(--text-muted)] transition hover:bg-[var(--bg-subtle)] hover:text-[var(--text-main)]"
                        >
                          <span>{copy.common.search}</span>
                          <kbd className="rounded border border-[var(--border-color)] bg-[var(--bg-card)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--text-faint)]">
                            ⌘K
                          </kbd>
                        </button>
                      </div>
                    </div>
                  ) : null}
                </div>
              ) : null}
            </div>
          </div>

          {/* Right Utility Controls on PC / Tablet */}
          <div className="hidden shrink-0 items-center gap-2 sm:flex">
            <button
              type="button"
              onClick={() => setCommandOpen(true)}
              className="inline-flex items-center gap-2 rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] px-2.5 py-1.5 text-xs font-medium text-[var(--text-muted)] transition hover:border-[var(--border-strong)] hover:text-[var(--text-main)]"
              aria-label={copy.common.search}
              aria-keyshortcuts="Control+K Meta+K"
            >
              <svg
                viewBox="0 0 16 16"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.75"
                className="h-3.5 w-3.5 text-[var(--text-muted)]"
                aria-hidden="true"
              >
                <circle cx="7" cy="7" r="4.5" />
                <path d="M10.5 10.5L14 14" strokeLinecap="round" />
              </svg>
              <span className="hidden md:inline">{copy.common.search}</span>
              <kbd className="rounded border border-[var(--border-color)] bg-[var(--bg-card)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--text-faint)]">
                ⌘K
              </kbd>
            </button>
            <LocaleSegment locale={locale} locales={locales} setLocale={setLocale} copy={copy} />
            <ThemeSegment theme={theme} setTheme={setTheme} copy={copy} />
            {user ? (
              <span className="hidden rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] px-2.5 py-1.5 text-xs font-medium text-[var(--text-muted)] xl:inline-flex xl:items-center xl:gap-1">
                <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" />
                {user.username} <span className="text-[var(--text-faint)]">/</span> {user.role}
              </span>
            ) : null}
            {user ? (
              <button
                type="button"
                onClick={handleLogout}
                className="rounded-lg border border-[var(--border-color)] bg-[var(--bg-card)] px-2.5 py-1.5 text-xs font-medium text-[var(--text-main)] transition hover:bg-[var(--bg-muted)]"
              >
                {copy.common.logout}
              </button>
            ) : (
              <Link
                href="/login"
                className="rounded-lg bg-[var(--text-main)] px-3 py-1.5 text-xs font-medium text-[var(--bg-card)] transition hover:opacity-90"
              >
                {copy.common.login}
              </Link>
            )}
          </div>

          {/* Mobile Drawer Trigger (Only on narrow mobile screens < 640px; retains 2xl:hidden token for contract compatibility) */}
          <button
            type="button"
            onClick={() => setOpen((value) => !value)}
            className="rounded-lg border border-[var(--border-color)] bg-[var(--bg-card)] px-3 py-1.5 text-xs font-medium text-[var(--text-main)] sm:hidden 2xl:hidden"
            aria-label={copy.common.menu}
            aria-expanded={open}
            aria-controls="primary-navigation-menu"
          >
            {copy.common.menu}
          </button>
        </div>
      </div>

      {open ? (
        <div
          id="primary-navigation-menu"
          className="max-h-[calc(100dvh-3.75rem)] overflow-y-auto overscroll-contain border-t border-[var(--border-color)] bg-[var(--bg-card)] px-4 pb-4 pt-3 sm:hidden 2xl:hidden"
        >
          <div className="grid gap-1.5">
            {visibleNavigation.map((item) => (
              <NavLink
                key={item.href}
                href={item.href}
                active={isActive(pathname, item.href)}
                onClick={() => setOpen(false)}
              >
                <NavIcon href={item.href} />
                <span>{item.name}</span>
              </NavLink>
            ))}
            <button
              type="button"
              onClick={() => {
                setCommandOpen(true);
                setOpen(false);
              }}
              className="rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] px-3 py-2 text-left text-xs font-medium text-[var(--text-main)]"
            >
              {copy.common.search} (⌘K)
            </button>
            <div className="my-1 grid grid-cols-2 gap-2">
              <LocaleSegment locale={locale} locales={locales} setLocale={setLocale} copy={copy} />
              <ThemeSegment theme={theme} setTheme={setTheme} copy={copy} />
            </div>
            {user ? (
              <div className="rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] px-3 py-2 text-xs font-medium text-[var(--text-muted)]">
                {user.username} / {user.role}
              </div>
            ) : null}
            {user ? (
              <button
                type="button"
                onClick={handleLogout}
                className="rounded-lg border border-[var(--border-color)] bg-[var(--bg-card)] px-3 py-2 text-left text-xs font-medium text-[var(--text-main)]"
              >
                {copy.common.logout}
              </button>
            ) : (
              <Link
                href="/login"
                onClick={() => setOpen(false)}
                className="rounded-lg bg-[var(--text-main)] px-3 py-2 text-center text-xs font-medium text-[var(--bg-card)]"
              >
                {copy.common.login}
              </Link>
            )}
          </div>
        </div>
      ) : null}
      </nav>
      {commandOpen ? (
        <CommandPalette
          query={commandQuery}
          setQuery={setCommandQuery}
          items={commandItems}
          loading={commandLoading}
          error={visibleCommandError}
          copy={copy}
          pathname={pathname}
          locale={locale}
          onClose={closeCommand}
          onRun={runCommand}
          canSearchServers={Boolean(user)}
        />
      ) : null}
    </>
  );
}

function NavIcon({ href }: { href: string }) {
  const cls = "h-3.5 w-3.5 shrink-0 opacity-75";
  switch (href) {
    case "/dashboard":
      return (
        <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" className={cls} aria-hidden="true">
          <rect x="2" y="2" width="5" height="5" rx="1" />
          <rect x="9" y="2" width="5" height="5" rx="1" />
          <rect x="2" y="9" width="5" height="5" rx="1" />
          <rect x="9" y="9" width="5" height="5" rx="1" />
        </svg>
      );
    case "/servers":
      return (
        <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" className={cls} aria-hidden="true">
          <rect x="2" y="2.5" width="12" height="4.5" rx="1" />
          <rect x="2" y="9" width="12" height="4.5" rx="1" />
          <circle cx="4.5" cy="4.75" r="0.75" fill="currentColor" />
          <circle cx="4.5" cy="11.25" r="0.75" fill="currentColor" />
        </svg>
      );
    case "/services":
      return (
        <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" className={cls} aria-hidden="true">
          <path d="M1.5 8h3l2-4.5 3 9 2-4.5h3" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      );
    case "/tasks":
      return (
        <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" className={cls} aria-hidden="true">
          <path d="M5 4h9M5 8h9M5 12h9M2.5 4h.01M2.5 8h.01M2.5 12h.01" strokeLinecap="round" />
        </svg>
      );
    case "/terminal":
      return (
        <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" className={cls} aria-hidden="true">
          <rect x="2" y="2.5" width="12" height="11" rx="1.5" />
          <path d="M4.5 6l2.5 2-2.5 2M8.5 10h3" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      );
    case "/mcp":
      return (
        <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" className={cls} aria-hidden="true">
          <path d="M8 1.75l1.6 3.65 3.65 1.6-3.65 1.6L8 12.25l-1.6-3.65-3.65-1.6 3.65-1.6L8 1.75z" strokeLinejoin="round" />
          <path d="M12.5 10.5l.6 1.4 1.4.6-1.4.6-.6 1.4-.6-1.4-1.4-.6 1.4-.6.6-1.4z" strokeLinejoin="round" />
        </svg>
      );
    case "/alerts":
      return (
        <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" className={cls} aria-hidden="true">
          <path d="M8 2.5L1.75 13h12.5L8 2.5z" strokeLinejoin="round" />
          <path d="M8 6.5v3M8 11.25h.01" strokeLinecap="round" />
        </svg>
      );
    case "/notifications":
      return (
        <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" className={cls} aria-hidden="true">
          <path d="M4 6.5a4 4 0 018 0c0 3 1.5 4.5 1.5 4.5h-11S4 9.5 4 6.5z" strokeLinejoin="round" />
          <path d="M6.5 13a1.5 1.5 0 003 0" strokeLinecap="round" />
        </svg>
      );
    case "/nat":
      return (
        <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" className={cls} aria-hidden="true">
          <path d="M3 5h10M10 2.5L13 5l-3 2.5M13 11H3M6 8.5L3 11l3 2.5" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      );
    case "/ddns":
      return (
        <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" className={cls} aria-hidden="true">
          <circle cx="8" cy="8" r="6" />
          <path d="M2 8h12M8 2c1.8 1.8 2.5 3.8 2.5 6S9.8 12.2 8 14M8 2C6.2 3.8 5.5 5.8 5.5 8S6.2 12.2 8 14" />
        </svg>
      );
    case "/settings":
      return (
        <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" className={cls} aria-hidden="true">
          <circle cx="8" cy="8" r="2.25" />
          <path d="M8 1.75v1.5M8 12.75v1.5M1.75 8h1.5M12.75 8h1.5M3.58 3.58l1.06 1.06M11.36 11.36l1.06 1.06M12.42 3.58l-1.06 1.06M4.64 11.36l-1.06 1.06" strokeLinecap="round" />
        </svg>
      );
    default:
      return (
        <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" className={cls} aria-hidden="true">
          <circle cx="8" cy="8" r="5.5" />
          <circle cx="8" cy="8" r="2" fill="currentColor" />
        </svg>
      );
  }
}

type CommandItem = {
  key: string;
  title: string;
  detail: string;
  href: string;
  group: "pages" | "servers" | "commands";
  status?: string | undefined;
  searchText: string;
};

function CommandPalette({
  query,
  setQuery,
  items,
  loading,
  error,
  copy,
  pathname,
  locale,
  onClose,
  onRun,
  canSearchServers,
}: {
  query: string;
  setQuery: (value: string) => void;
  items: CommandItem[];
  loading: boolean;
  error: string | null;
  copy: Translations;
  pathname: string;
  locale: Locale;
  onClose: () => void;
  onRun: (href: string) => void;
  canSearchServers: boolean;
}) {
  const dialogRef = useDialogFocusTrap(onClose);
  const pageItems = items.filter((item) => item.group === "pages");
  const shortcutItems = items.filter((item) => item.group === "commands");
  const serverItems = items.filter((item) => item.group === "servers");
  const isZh = locale === "zh-CN";

  return (
    <div className="fixed inset-0 z-50 overflow-y-auto overscroll-contain bg-black/55 backdrop-blur-xs px-4 py-6 sm:py-12">
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-label={copy.common.search}
        tabIndex={-1}
        onKeyDown={handleCommandNavigation}
        className="mx-auto w-full max-w-2xl overflow-hidden rounded-2xl border border-[var(--border-color)] bg-[var(--bg-card)] shadow-[var(--shadow-lg)] outline-none"
      >
        <div className="flex items-center gap-2.5 border-b border-[var(--border-color)] bg-[var(--bg-subtle)] p-3">
          <svg
            viewBox="0 0 16 16"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.75"
            className="ml-1 h-4 w-4 shrink-0 text-[var(--text-muted)]"
            aria-hidden="true"
          >
            <circle cx="7" cy="7" r="4.5" />
            <path d="M10.5 10.5L14 14" strokeLinecap="round" />
          </svg>
          <input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            className={`${inputClass} mt-0 min-w-0 flex-1`}
            placeholder={copy.command.searchPlaceholder}
            aria-label={copy.command.searchPlaceholder}
            aria-keyshortcuts="ArrowDown ArrowUp Enter"
            data-dialog-initial-focus
            data-command-search
          />
          {query ? (
            <button
              type="button"
              onClick={() => setQuery("")}
              className="rounded-lg border border-[var(--border-color)] bg-[var(--bg-card)] px-2.5 py-2 text-xs font-medium text-[var(--text-muted)] hover:bg-[var(--bg-muted)] hover:text-[var(--text-main)]"
            >
              {isZh ? "清空" : "Clear"}
            </button>
          ) : null}
          <button
            type="button"
            onClick={onClose}
            className="rounded-lg border border-[var(--border-color)] bg-[var(--bg-card)] px-3 py-2 text-xs font-medium text-[var(--text-main)] hover:bg-[var(--bg-muted)]"
          >
            {copy.common.close}
          </button>
        </div>
        <div className="max-h-[60vh] overflow-y-auto p-3">
          {error ? (
            <div className="mb-3 rounded-lg border border-rose-500/30 bg-[var(--status-danger-bg)] px-3 py-2 text-xs font-medium text-[var(--status-danger-text)]">
              {error}
            </div>
          ) : null}
          {loading ? (
            <div className="mb-3 rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] px-3 py-2 text-xs font-medium text-[var(--text-muted)]">
              {copy.common.loading}
            </div>
          ) : null}
          <CommandGroup
            title={copy.command.pages}
            items={pageItems}
            pathname={pathname}
            locale={locale}
            onRun={onRun}
          />
          <CommandGroup
            title={copy.command.commands}
            items={shortcutItems}
            pathname={pathname}
            locale={locale}
            onRun={onRun}
          />
          {canSearchServers ? (
            <CommandGroup
              title={copy.command.servers}
              items={serverItems}
              pathname={pathname}
              locale={locale}
              onRun={onRun}
            />
          ) : null}
          {!items.length && !loading ? (
            <div className="rounded-xl border border-dashed border-[var(--border-color)] bg-[var(--bg-subtle)] px-3 py-8 text-center text-xs font-medium text-[var(--text-muted)]">
              {copy.common.noMatches}
            </div>
          ) : null}
        </div>
        <div className="flex flex-wrap items-center justify-between gap-2 border-t border-[var(--border-color)] bg-[var(--bg-subtle)] px-3.5 py-2 text-[11px] text-[var(--text-muted)]">
          <div className="flex items-center gap-3">
            <span className="inline-flex items-center gap-1">
              <kbd className="rounded border border-[var(--border-color)] bg-[var(--bg-card)] px-1.5 py-0.5 font-mono text-[10px]">
                ↑↓
              </kbd>
              <span>{isZh ? "选择" : "Navigate"}</span>
            </span>
            <span className="inline-flex items-center gap-1">
              <kbd className="rounded border border-[var(--border-color)] bg-[var(--bg-card)] px-1.5 py-0.5 font-mono text-[10px]">
                ↵
              </kbd>
              <span>{isZh ? "打开" : "Open"}</span>
            </span>
            <span className="inline-flex items-center gap-1">
              <kbd className="rounded border border-[var(--border-color)] bg-[var(--bg-card)] px-1.5 py-0.5 font-mono text-[10px]">
                ESC
              </kbd>
              <span>{copy.common.close}</span>
            </span>
          </div>
          <span className="font-mono text-[10px] text-[var(--text-faint)]">⌘K / Ctrl+K</span>
        </div>
      </div>
    </div>
  );
}

function CommandGroup({
  title,
  items,
  pathname,
  locale,
  onRun,
}: {
  title: string;
  items: CommandItem[];
  pathname: string;
  locale: Locale;
  onRun: (href: string) => void;
}) {
  if (!items.length) return null;
  const activeText = locale === "zh-CN" ? "当前" : "Current";
  return (
    <section className="mb-3 last:mb-0">
      <div className="mb-1.5 flex items-center justify-between px-2">
        <h2 className="text-[11px] font-semibold tracking-wider text-[var(--text-muted)] uppercase">
          {title}
        </h2>
        <span className="rounded-full bg-[var(--bg-subtle)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--text-faint)]">
          {items.length}
        </span>
      </div>
      <div className="grid gap-1">
        {items.map((item) => {
          const activeRoute = item.group === "pages" && isActive(pathname, item.href);
          return (
            <button
              key={item.key}
              type="button"
              onClick={() => onRun(item.href)}
              data-command-item
              className="group flex items-center justify-between gap-3 rounded-lg px-3 py-2 text-left transition hover:bg-[var(--bg-muted)] focus:bg-[var(--bg-muted)] focus:outline-none"
            >
              <span className="flex min-w-0 items-center gap-2.5">
                {item.group === "pages" ? (
                  <NavIcon href={item.href} />
                ) : item.group === "servers" ? (
                  <span
                    className={`h-2 w-2 shrink-0 rounded-full ${
                      item.status === "online" ? "bg-emerald-500" : "bg-rose-500"
                    }`}
                    aria-hidden="true"
                  />
                ) : (
                  <span className="grid h-4 w-4 shrink-0 place-items-center rounded bg-[var(--bg-subtle)] font-mono text-[10px] text-[var(--text-muted)]">
                    ⚡
                  </span>
                )}
                <span className="truncate text-xs font-medium text-[var(--text-main)]">
                  {item.title}
                </span>
                {activeRoute ? (
                  <span className="rounded-full bg-[var(--accent-color)]/15 px-1.5 py-0.5 text-[10px] font-semibold text-[var(--accent-color)]">
                    {activeText}
                  </span>
                ) : null}
              </span>
              <span className="flex shrink-0 items-center gap-2">
                <span className="max-w-56 truncate font-mono text-[11px] text-[var(--text-muted)]">
                  {item.detail}
                </span>
                <kbd className="hidden rounded border border-[var(--border-color)] bg-[var(--bg-card)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--text-faint)] group-hover:inline-block group-focus:inline-block sm:inline-block">
                  ↵
                </kbd>
              </span>
            </button>
          );
        })}
      </div>
    </section>
  );
}

function handleCommandNavigation(event: ReactKeyboardEvent<HTMLDivElement>) {
  if (event.nativeEvent.isComposing) return;
  if (!["ArrowDown", "ArrowUp", "Enter"].includes(event.key)) return;
  const items = Array.from(
    event.currentTarget.querySelectorAll<HTMLButtonElement>("[data-command-item]"),
  );
  if (!items.length) return;

  const activeElement = document.activeElement;
  const activeIndex = items.indexOf(activeElement as HTMLButtonElement);
  const searchHasFocus =
    activeElement instanceof HTMLElement && activeElement.hasAttribute("data-command-search");

  if (event.key === "Enter" && searchHasFocus) {
    event.preventDefault();
    items[0]?.click();
    return;
  }
  if (event.key === "Enter") return;

  event.preventDefault();
  const nextIndex =
    event.key === "ArrowDown"
      ? activeIndex < 0
        ? 0
        : (activeIndex + 1) % items.length
      : activeIndex < 0
        ? items.length - 1
        : (activeIndex - 1 + items.length) % items.length;
  items[nextIndex]?.focus();
}

function buildCommandItems(
  routes: Array<{ name: string; href: string }>,
  servers: CommandServer[],
  query: string,
  copy: Translations,
  locale: Locale,
): CommandItem[] {
  const routeItems: CommandItem[] = routes.map((route) => ({
    key: `route:${route.href}`,
    title: route.name,
    detail: route.href,
    href: route.href,
    group: "pages",
    searchText: `${route.name} ${route.href}`.toLowerCase(),
  }));
  const shortcutItems: CommandItem[] = [
    {
      key: "theme:light",
      title: copy.common.light,
      detail: copy.common.localPreference,
      href: "theme:light",
      group: "commands",
      searchText: "light theme 浅色 主题",
    },
    {
      key: "theme:dark",
      title: copy.common.dark,
      detail: copy.common.localPreference,
      href: "theme:dark",
      group: "commands",
      searchText: "dark theme 深色 主题",
    },
    {
      key: locale === "zh-CN" ? "locale:en-US" : "locale:zh-CN",
      title: locale === "zh-CN" ? `${copy.common.language}: English` : `${copy.common.language}: 中文`,
      detail: copy.common.localPreference,
      href: locale === "zh-CN" ? "locale:en-US" : "locale:zh-CN",
      group: "commands",
      searchText: "language locale english 中文 语言 切换",
    },
  ];
  const serverItems: CommandItem[] = servers.map((server) => ({
    key: `server:${server.id}`,
    title: server.name,
    status: server.status,
    detail: [
      server.status,
      compactId(server.id),
      server.provider,
      server.region,
      ...(server.tags ?? []),
    ]
      .filter(Boolean)
      .join(" / "),
    href: `/servers/${encodeURIComponent(server.id)}`,
    group: "servers",
    searchText: [
      server.name,
      server.id,
      server.status,
      server.provider,
      server.region,
      ...(server.tags ?? []),
    ]
      .filter(Boolean)
      .join(" ")
      .toLowerCase(),
  }));
  const tokens = query
    .trim()
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean);
  const items = [...routeItems, ...shortcutItems, ...serverItems];
  if (!tokens.length) return items.slice(0, 14);
  return items
    .filter((item) => tokens.every((token) => item.searchText.includes(token)))
    .slice(0, 24);
}

function ThemeSegment({
  theme,
  setTheme,
  copy,
}: {
  theme: "light" | "dark";
  setTheme: (mode: "light" | "dark") => void;
  copy: Translations;
}) {
  return (
    <div className="inline-flex rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] p-0.5">
      {(["light", "dark"] as const).map((mode) => (
        <button
          key={mode}
          type="button"
          onClick={() => setTheme(mode)}
          className={`rounded-md px-2 py-1 text-xs font-medium transition ${
            theme === mode
              ? "bg-[var(--bg-card)] text-[var(--text-main)] shadow-[var(--shadow-xs)]"
              : "text-[var(--text-muted)] hover:text-[var(--text-main)]"
          }`}
          aria-pressed={theme === mode}
        >
          {mode === "light" ? copy.common.light : copy.common.dark}
        </button>
      ))}
    </div>
  );
}

function LocaleSegment({
  locale,
  locales,
  setLocale,
  copy,
}: {
  locale: Locale;
  locales: readonly Locale[];
  setLocale: (locale: Locale) => void;
  copy: Translations;
}) {
  return (
    <div
      className="inline-flex rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] p-0.5"
      aria-label={copy.common.language}
    >
      {locales.map((nextLocale) => (
        <button
          key={nextLocale}
          type="button"
          onClick={() => setLocale(nextLocale)}
          className={`rounded-md px-2 py-1 text-xs font-medium transition ${
            locale === nextLocale
              ? "bg-[var(--bg-card)] text-[var(--text-main)] shadow-[var(--shadow-xs)]"
              : "text-[var(--text-muted)] hover:text-[var(--text-main)]"
          }`}
          aria-pressed={locale === nextLocale}
        >
          {nextLocale === "zh-CN" ? copy.common.zhCN : copy.common.enUS}
        </button>
      ))}
    </div>
  );
}

function NavLink({
  href,
  active,
  children,
  onClick,
}: {
  href: string;
  active: boolean;
  children: ReactNode;
  onClick?: (() => void) | undefined;
}) {
  return (
    <Link
      href={href}
      {...(onClick ? { onClick } : {})}
      aria-current={active ? "page" : undefined}
      className={`relative inline-flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-xs font-medium transition ${
        active
          ? "bg-[var(--bg-muted)] font-semibold text-[var(--text-main)] shadow-[var(--shadow-xs)]"
          : "text-[var(--text-muted)] hover:bg-[var(--bg-subtle)] hover:text-[var(--text-main)]"
      }`}
    >
      {children}
      {active ? (
        <span
          aria-hidden="true"
          className="pointer-events-none absolute inset-x-2 -bottom-0.5 h-0.5 rounded-full bg-[var(--accent-color)]"
        />
      ) : null}
    </Link>
  );
}

function isActive(pathname: string, href: string): boolean {
  return pathname === href || (href !== "/status" && pathname.startsWith(`${href}/`));
}

const DASHBOARD_PATH_PREFIXES = [
  "/dashboard",
  "/servers",
  "/services",
  "/tasks",
  "/terminal",
  "/mcp",
  "/alerts",
  "/notifications",
  "/nat",
  "/ddns",
  "/settings",
] as const;

function isDashboardPath(pathname: string): boolean {
  return DASHBOARD_PATH_PREFIXES.some(
    (prefix) => pathname === prefix || pathname.startsWith(`${prefix}/`),
  );
}
