"use client";

import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { type KeyboardEvent as ReactKeyboardEvent, useEffect, useMemo, useRef, useState } from "react";
import { apiClient, type ThemeDefinition } from "@/lib/api";
import { type Locale, type Translations } from "@/lib/i18n";
import { useI18n } from "@/lib/use-i18n";
import { clearStoredUser, compactId, inputClass, isAdmin, readStoredUser, useBoldTheme, useDialogFocusTrap, useStoredUser } from "./M7Primitives";
import { useOptionalServerData } from "./ServerDataProvider";
import {
  applyDashboardTheme,
  cacheDashboardTheme,
  clearDashboardThemeCache,
  DASHBOARD_THEME_CACHE_KEY,
  DASHBOARD_THEME_EVENT,
  readCachedDashboardTheme,
} from "@/app/lib/dashboard-theme";

type NavItem = { name: string; href: string; adminOnly?: boolean };

function navigationFor(copy: Translations): NavItem[] {
  return [
    { name: copy.nav.dashboard, href: "/dashboard" },
    { name: copy.nav.servers, href: "/servers" },
    { name: copy.nav.services, href: "/services" },
    { name: copy.nav.tasks, href: "/tasks" },
    { name: copy.nav.terminal, href: "/terminal" },
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
  status?: string;
  tags?: string[];
  provider?: string | null;
  region?: string | null;
}

const EMPTY_COMMAND_SERVERS: CommandServer[] = [];

interface OwnedDashboardTheme {
  userId: string;
  theme: ThemeDefinition | null;
}

export default function Navigation() {
  const pathname = usePathname();
  const dashboardPath = isDashboardPath(pathname);
  const router = useRouter();
  const user = useStoredUser();
  const serverData = useOptionalServerData();
  const { locale, locales, setLocale, t: copy } = useI18n();
  const [theme, setTheme] = useBoldTheme();
  const [dashboardTheme, setDashboardTheme] = useState<OwnedDashboardTheme>(() => {
    const storedUser = readStoredUser();
    return {
      userId: storedUser?.id ?? "",
      theme: storedUser ? readCachedDashboardTheme(storedUser.id) : null,
    };
  });
  const [open, setOpen] = useState(false);
  const [commandOpen, setCommandOpen] = useState(false);
  const [commandQuery, setCommandQuery] = useState("");
  const [commandServers, setCommandServers] = useState<CommandServer[]>([]);
  const [commandLoadingUserId, setCommandLoadingUserId] = useState("");
  const [commandLoadedUserId, setCommandLoadedUserId] = useState("");
  const [commandError, setCommandError] = useState<{ userId: string; message: string } | null>(null);
  const commandRequestUserIdRef = useRef("");
  const dashboardThemeRevisionRef = useRef(0);
  const revalidateCommandServers = serverData?.revalidate;

  const handleLogout = async () => {
    await apiClient.logout();
    applyDashboardTheme(null);
    clearDashboardThemeCache();
    clearStoredUser();
    router.replace("/login");
  };

  const navigation = useMemo(() => navigationFor(copy), [copy]);
  const publicNavigation = useMemo(() => publicNavigationFor(copy), [copy]);
  const commandLoading = Boolean(user && (
    serverData ? serverData.loading : commandLoadingUserId === user.id
  ));
  const visibleCommandServers = user
    ? serverData?.servers ?? (commandLoadedUserId === user.id ? commandServers : EMPTY_COMMAND_SERVERS)
    : EMPTY_COMMAND_SERVERS;
  const fallbackCommandError = commandError && commandError.userId === user?.id
    ? commandError.message
    : null;
  const visibleCommandError = user ? serverData?.error ?? fallbackCommandError : null;
  const visibleNavigation = user
    ? navigation.filter((item) => !item.adminOnly || isAdmin(user))
    : publicNavigation;

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setCommandOpen((value) => !value);
      }
      if (event.key === "Escape") {
        setCommandOpen(false);
        setOpen(false);
      }
    }

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
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
      setCommandLoadingUserId((current) => current === requestUserId ? "" : current);
      if (response.success && response.data) {
        const responseServers = Array.isArray(response.data.servers) ? response.data.servers : [];
        setCommandLoadedUserId(requestUserId);
        setCommandServers(
          (responseServers as CommandServer[]).filter((server) => server.id && server.name),
        );
      } else {
        setCommandError({ userId: requestUserId, message: response.error || copy.common.requestFailed });
      }
    });
    return () => {
      controller.abort();
      if (commandRequestUserIdRef.current === requestUserId) commandRequestUserIdRef.current = "";
    };
  }, [commandOpen, copy.common.requestFailed, revalidateCommandServers, user]);

  useEffect(() => {
    if (!dashboardPath) {
      applyDashboardTheme(null);
      return;
    }
    if (!user) {
      // useStoredUser hydrates from a null server snapshot. Keep the inline
      // cached theme intact until the client identity snapshot arrives.
      if (readStoredUser()) return;
      applyDashboardTheme(null);
      clearDashboardThemeCache();
      return;
    }
    const cachedTheme = readCachedDashboardTheme(user.id);
    applyDashboardTheme(cachedTheme);
    let cancelled = false;
    const requestRevision = dashboardThemeRevisionRef.current;
    apiClient.listThemes({ bypassClientCache: true }).then((response) => {
      if (cancelled || requestRevision !== dashboardThemeRevisionRef.current) return;
      if (response.success && response.data) {
        const selected = response.data.themes.find(
          (item) => item.id === response.data?.selected_dashboard_theme_id,
        );
        const nextTheme = selected ?? null;
        cacheDashboardTheme(user.id, nextTheme);
        setDashboardTheme({ userId: user.id, theme: nextTheme });
        applyDashboardTheme(nextTheme);
      }
    });
    return () => {
      cancelled = true;
    };
  }, [dashboardPath, user]);

  useEffect(() => {
    if (!dashboardPath) return;
    return () => applyDashboardTheme(null);
  }, [dashboardPath]);

  useEffect(() => {
    if (!user) return;
    const handleThemeChange = (event: Event) => {
      if (event.type === "storage") {
        const storageKey = (event as StorageEvent).key;
        if (storageKey === "user") {
          apiClient.invalidateCachedReads();
          return;
        }
        if (storageKey !== DASHBOARD_THEME_CACHE_KEY) return;
        apiClient.invalidateCachedReads();
      }
      const detail = (event as CustomEvent<{
        userId?: string;
        theme?: ThemeDefinition | null;
      }>).detail;
      if (event.type !== "storage" && detail?.userId && detail.userId !== user.id) return;
      dashboardThemeRevisionRef.current += 1;
      const nextTheme = detail && "theme" in detail
        ? detail.theme ?? null
        : readCachedDashboardTheme(user.id);
      setDashboardTheme({ userId: user.id, theme: nextTheme });
      if (dashboardPath) applyDashboardTheme(nextTheme);
    };
    window.addEventListener(DASHBOARD_THEME_EVENT, handleThemeChange);
    window.addEventListener("storage", handleThemeChange);
    return () => {
      window.removeEventListener(DASHBOARD_THEME_EVENT, handleThemeChange);
      window.removeEventListener("storage", handleThemeChange);
    };
  }, [dashboardPath, user]);

  useEffect(() => {
    if (!dashboardPath) {
      applyDashboardTheme(null);
      return;
    }
    if (!user) {
      if (!readStoredUser()) applyDashboardTheme(null);
      return;
    }
    const nextTheme = dashboardTheme.userId === user.id
      ? dashboardTheme.theme
      : readCachedDashboardTheme(user.id);
    applyDashboardTheme(nextTheme);
  }, [dashboardPath, dashboardTheme, theme, user]);

  const commandItems = useMemo(
    () => buildCommandItems(visibleNavigation, visibleCommandServers, commandQuery, copy),
    [commandQuery, copy, visibleCommandServers, visibleNavigation],
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
    closeCommand();
    setOpen(false);
    router.push(href);
  }

  return (
    <nav className="sticky top-0 z-40 border-b-4 border-black bg-[var(--bg-card)]">
      <div className="mx-auto max-w-[96rem] px-4 sm:px-6 lg:px-8">
        <div className="flex min-h-16 items-center justify-between gap-4 py-3">
          <Link href="/status" className="group flex items-center gap-3">
            <span className="grid h-10 w-10 place-items-center border-2 border-black bg-[var(--accent-color)] text-lg font-black text-white shadow-[var(--shadow-brutal-sm)] group-hover:-translate-x-0.5 group-hover:-translate-y-0.5 group-hover:shadow-[var(--shadow-brutal)]">
              XL
            </span>
            <span className="text-xl font-black uppercase tracking-tight text-[var(--text-main)]">
              XLStatus
            </span>
          </Link>

          <div className="hidden flex-1 items-center justify-center 2xl:flex">
            <div className="flex flex-wrap justify-center gap-2">
              {visibleNavigation.map((item) => (
                <NavLink key={item.href} href={item.href} active={isActive(pathname, item.href)}>
                  {item.name}
                </NavLink>
              ))}
            </div>
          </div>

          <div className="hidden items-center gap-2 2xl:flex">
            <button
              type="button"
              onClick={() => setCommandOpen(true)}
              className="border-2 border-black bg-[var(--bg-card)] px-3 py-2 text-xs font-black uppercase shadow-[var(--shadow-brutal-sm)]"
              aria-label={copy.common.search}
              aria-keyshortcuts="Control+K Meta+K"
            >
              {copy.common.search}
            </button>
            <LocaleSegment locale={locale} locales={locales} setLocale={setLocale} copy={copy} />
            <ThemeSegment theme={theme} setTheme={setTheme} copy={copy} />
            {user ? (
              <span className="border-2 border-black bg-[var(--accent-bg)] px-3 py-2 text-xs font-black uppercase shadow-[var(--shadow-brutal-sm)]">
                {user.username} / {user.role}
              </span>
            ) : null}
            {user ? (
              <button
                type="button"
                onClick={handleLogout}
                className="border-2 border-black bg-black px-3 py-2 text-xs font-black uppercase text-white shadow-[var(--shadow-brutal-sm)]"
              >
                {copy.common.logout}
              </button>
            ) : (
              <Link
                href="/login"
                className="border-2 border-black bg-black px-3 py-2 text-xs font-black uppercase text-white shadow-[var(--shadow-brutal-sm)]"
              >
                {copy.common.login}
              </Link>
            )}
          </div>

          <button
            type="button"
            onClick={() => setOpen((value) => !value)}
            className="border-2 border-black bg-[var(--bg-card)] px-3 py-2 text-sm font-black uppercase shadow-[var(--shadow-brutal-sm)] 2xl:hidden"
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
          className="max-h-[calc(100dvh-4.25rem)] overflow-y-auto overscroll-contain border-t-4 border-black bg-[var(--bg-card)] px-4 pb-4 pt-3 2xl:hidden"
        >
          <div className="grid gap-2">
            {visibleNavigation.map((item) => (
              <NavLink
                key={item.href}
                href={item.href}
                active={isActive(pathname, item.href)}
                onClick={() => setOpen(false)}
              >
                {item.name}
              </NavLink>
            ))}
            <button
              type="button"
              onClick={() => {
                setCommandOpen(true);
                setOpen(false);
              }}
              className="border-2 border-black bg-[var(--bg-card)] px-3 py-2 text-left text-xs font-black uppercase"
            >
              {copy.common.search}
            </button>
            <LocaleSegment locale={locale} locales={locales} setLocale={setLocale} copy={copy} />
            <ThemeSegment theme={theme} setTheme={setTheme} copy={copy} />
            {user ? (
              <div className="border-2 border-black bg-[var(--accent-bg)] px-3 py-2 text-xs font-black uppercase">
                {user.username} / {user.role}
              </div>
            ) : null}
            {user ? (
              <button
                type="button"
                onClick={handleLogout}
                className="border-2 border-black bg-black px-3 py-2 text-left text-xs font-black uppercase text-white"
              >
                {copy.common.logout}
              </button>
            ) : (
              <Link
                href="/login"
                onClick={() => setOpen(false)}
                className="border-2 border-black bg-black px-3 py-2 text-xs font-black uppercase text-white"
              >
                {copy.common.login}
              </Link>
            )}
          </div>
        </div>
      ) : null}
      {commandOpen ? (
        <CommandPalette
          query={commandQuery}
          setQuery={setCommandQuery}
          items={commandItems}
          loading={commandLoading}
          error={visibleCommandError}
          copy={copy}
          onClose={closeCommand}
          onRun={runCommand}
          canSearchServers={Boolean(user)}
        />
      ) : null}
    </nav>
  );
}

type CommandItem = {
  key: string;
  title: string;
  detail: string;
  href: string;
  group: "pages" | "servers" | "commands";
  searchText: string;
};

function CommandPalette({
  query,
  setQuery,
  items,
  loading,
  error,
  copy,
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
  onClose: () => void;
  onRun: (href: string) => void;
  canSearchServers: boolean;
}) {
  const dialogRef = useDialogFocusTrap(onClose);
  const pageItems = items.filter((item) => item.group === "pages");
  const shortcutItems = items.filter((item) => item.group === "commands");
  const serverItems = items.filter((item) => item.group === "servers");

  return (
    <div className="fixed inset-0 z-50 overflow-y-auto overscroll-contain bg-black/50 px-4 py-4 sm:py-10">
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-label={copy.common.search}
        tabIndex={-1}
        onKeyDown={handleCommandNavigation}
        className="mx-auto w-full max-w-2xl border-4 border-black bg-[var(--bg-card)] shadow-[10px_10px_0_0_#000] outline-none"
      >
        <div className="flex items-center gap-3 border-b-4 border-black bg-[var(--accent-bg)] p-3">
          <input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            className={`${inputClass} min-w-0 flex-1`}
            placeholder={copy.command.searchPlaceholder}
            aria-label={copy.command.searchPlaceholder}
            aria-keyshortcuts="ArrowDown ArrowUp Enter"
            data-dialog-initial-focus
            data-command-search
          />
          <button type="button" onClick={onClose} className="border-2 border-black bg-black px-3 py-2 text-xs font-black uppercase text-white shadow-[var(--shadow-brutal-sm)]">
            {copy.common.close}
          </button>
        </div>
        <div className="max-h-[60vh] overflow-y-auto p-3">
          {error ? (
            <div className="mb-3 border-2 border-black bg-[var(--btn-bg)] px-3 py-2 text-sm font-bold text-[var(--btn-text)]">
              {error}
            </div>
          ) : null}
          {loading ? (
            <div className="border-2 border-black bg-[var(--accent-bg)] px-3 py-2 text-sm font-black">
              {copy.common.loading}
            </div>
          ) : null}
          <CommandGroup title={copy.command.pages} items={pageItems} onRun={onRun} />
          <CommandGroup title={copy.command.commands} items={shortcutItems} onRun={onRun} />
          {canSearchServers ? <CommandGroup title={copy.command.servers} items={serverItems} onRun={onRun} /> : null}
          {!items.length && !loading ? (
            <div className="border-2 border-black bg-[var(--accent-bg)] px-3 py-8 text-center text-sm font-black">
              {copy.common.noMatches}
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
}

function CommandGroup({
  title,
  items,
  onRun,
}: {
  title: string;
  items: CommandItem[];
  onRun: (href: string) => void;
}) {
  if (!items.length) return null;
  return (
    <section className="mb-3 last:mb-0">
      <h2 className="mb-2 border-2 border-black bg-[var(--accent-bg)] px-2 py-1 text-xs font-black uppercase shadow-[var(--shadow-brutal-sm)]">
        {title}
      </h2>
      <div className="grid gap-2">
        {items.map((item) => (
          <button
            key={item.key}
            type="button"
            onClick={() => onRun(item.href)}
            data-command-item
            className="grid gap-1 border-2 border-black bg-[var(--bg-card)] px-3 py-2 text-left shadow-[var(--shadow-brutal-sm)] transition hover:-translate-x-0.5 hover:-translate-y-0.5 hover:bg-[var(--accent-bg)]"
          >
            <span className="text-sm font-black text-[var(--text-main)]">{item.title}</span>
            <span className="break-all text-xs font-bold text-[var(--text-muted)]">{item.detail}</span>
          </button>
        ))}
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
  const searchHasFocus = activeElement instanceof HTMLElement
    && activeElement.hasAttribute("data-command-search");

  if (event.key === "Enter" && searchHasFocus) {
    event.preventDefault();
    items[0].click();
    return;
  }
  if (event.key === "Enter") return;

  event.preventDefault();
  const nextIndex = event.key === "ArrowDown"
    ? activeIndex < 0 ? 0 : (activeIndex + 1) % items.length
    : activeIndex < 0 ? items.length - 1 : (activeIndex - 1 + items.length) % items.length;
  items[nextIndex].focus();
}

function buildCommandItems(
  routes: Array<{ name: string; href: string }>,
  servers: CommandServer[],
  query: string,
  copy: Translations,
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
  ];
  const serverItems: CommandItem[] = servers.map((server) => ({
    key: `server:${server.id}`,
    title: server.name,
    detail: [server.status, compactId(server.id), server.provider, server.region, ...(server.tags ?? [])]
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
  const needle = query.trim().toLowerCase();
  const items = [...routeItems, ...shortcutItems, ...serverItems];
  if (!needle) return items.slice(0, 12);
  return items.filter((item) => item.searchText.includes(needle)).slice(0, 24);
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
    <div className="grid grid-cols-2 border-2 border-black bg-[var(--bg-card)] shadow-[var(--shadow-brutal-sm)]">
      {(["light", "dark"] as const).map((mode) => (
        <button
          key={mode}
          type="button"
          onClick={() => setTheme(mode)}
          className={`px-3 py-2 text-xs font-black uppercase ${
            theme === mode
              ? "bg-[var(--accent-color)] text-[var(--btn-text)]"
              : "bg-[var(--bg-card)] text-[var(--text-main)]"
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
    <div className="grid grid-cols-2 border-2 border-black bg-[var(--bg-card)] shadow-[var(--shadow-brutal-sm)]" aria-label={copy.common.language}>
      {locales.map((nextLocale) => (
        <button
          key={nextLocale}
          type="button"
          onClick={() => setLocale(nextLocale)}
          className={`px-3 py-2 text-xs font-black uppercase ${
            locale === nextLocale
              ? "bg-[var(--accent-color)] text-[var(--btn-text)]"
              : "bg-[var(--bg-card)] text-[var(--text-main)]"
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
  children: React.ReactNode;
  onClick?: () => void;
}) {
  return (
    <Link
      href={href}
      onClick={onClick}
      className={`border-2 border-black px-3 py-2 text-xs font-black uppercase shadow-[var(--shadow-brutal-sm)] transition hover:-translate-x-0.5 hover:-translate-y-0.5 ${
        active
          ? "bg-[var(--accent-color)] text-white"
          : "bg-[var(--bg-card)] text-[var(--text-main)]"
      }`}
    >
      {children}
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
