"use client";

import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { useEffect, useMemo, useRef, useState } from "react";
import { apiClient, type ThemeDefinition } from "@/lib/api";
import { type Locale, type Translations } from "@/lib/i18n";
import { useI18n } from "@/lib/use-i18n";
import { compactId, inputClass, isAdmin, useBoldTheme, useStoredUser } from "./M7Primitives";

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

export default function Navigation() {
  const pathname = usePathname();
  const router = useRouter();
  const user = useStoredUser();
  const { locale, locales, setLocale, t: copy } = useI18n();
  const [theme, setTheme] = useBoldTheme();
  const [open, setOpen] = useState(false);
  const [commandOpen, setCommandOpen] = useState(false);
  const [commandQuery, setCommandQuery] = useState("");
  const [commandServers, setCommandServers] = useState<CommandServer[]>([]);
  const [commandLoading, setCommandLoading] = useState(false);
  const [commandLoaded, setCommandLoaded] = useState(false);
  const [commandError, setCommandError] = useState<string | null>(null);
  const commandInputRef = useRef<HTMLInputElement | null>(null);

  const handleLogout = async () => {
    await apiClient.logout();
    localStorage.removeItem("session_token");
    localStorage.removeItem("user");
    window.location.href = "/login";
  };

  const navigation = useMemo(() => navigationFor(copy), [copy]);
  const publicNavigation = useMemo(() => publicNavigationFor(copy), [copy]);
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
      }
    }

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  useEffect(() => {
    if (!commandOpen) return;
    const timeoutId = window.setTimeout(() => commandInputRef.current?.focus(), 0);
    return () => window.clearTimeout(timeoutId);
  }, [commandOpen]);

  useEffect(() => {
    if (!commandOpen || !user || commandLoaded || commandLoading) return;
    let cancelled = false;
    const timeoutId = window.setTimeout(() => {
      setCommandLoading(true);
      setCommandError(null);
      apiClient.listServers(200, 0).then((response) => {
        if (cancelled) return;
        setCommandLoading(false);
        setCommandLoaded(true);
        if (response.success && response.data) {
          setCommandServers(
            ((response.data.servers as CommandServer[]) ?? []).filter((server) => server.id && server.name),
          );
        } else {
          setCommandError(response.error || copy.common.requestFailed);
        }
      });
    }, 0);
    return () => {
      cancelled = true;
      window.clearTimeout(timeoutId);
    };
  }, [commandLoaded, commandLoading, commandOpen, copy.common.requestFailed, user]);

  useEffect(() => {
    if (!user) {
      applyDashboardTheme(null);
      return;
    }
    let cancelled = false;
    apiClient.listThemes().then((response) => {
      if (cancelled) return;
      if (response.success && response.data) {
        const selected = response.data.themes.find(
          (item) => item.id === response.data?.selected_dashboard_theme_id,
        );
        applyDashboardTheme(selected ?? null);
      }
    });
    return () => {
      cancelled = true;
    };
  }, [user]);

  const commandItems = useMemo(
    () => buildCommandItems(visibleNavigation, commandServers, commandQuery, copy),
    [commandQuery, commandServers, copy, visibleNavigation],
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
      <div className="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
        <div className="flex min-h-16 items-center justify-between gap-4 py-3">
          <Link href="/status" className="group flex items-center gap-3">
            <span className="grid h-10 w-10 place-items-center border-2 border-black bg-[var(--accent-color)] text-lg font-black text-white shadow-[var(--shadow-brutal-sm)] group-hover:-translate-x-0.5 group-hover:-translate-y-0.5 group-hover:shadow-[var(--shadow-brutal)]">
              XL
            </span>
            <span className="text-xl font-black uppercase tracking-tight text-[var(--text-main)]">
              XLStatus
            </span>
          </Link>

          <div className="hidden flex-1 items-center justify-center md:flex">
            <div className="flex flex-wrap justify-center gap-2">
              {visibleNavigation.map((item) => (
                <NavLink key={item.href} href={item.href} active={isActive(pathname, item.href)}>
                  {item.name}
                </NavLink>
              ))}
            </div>
          </div>

          <div className="hidden items-center gap-2 md:flex">
            <button
              type="button"
              onClick={() => setCommandOpen(true)}
              className="border-2 border-black bg-[var(--bg-card)] px-3 py-2 text-xs font-black uppercase shadow-[var(--shadow-brutal-sm)]"
              aria-label={copy.common.search}
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
            className="border-2 border-black bg-[var(--bg-card)] px-3 py-2 text-sm font-black uppercase shadow-[var(--shadow-brutal-sm)] md:hidden"
            aria-label={copy.common.menu}
          >
            {copy.common.menu}
          </button>
        </div>
      </div>

      {open ? (
        <div className="border-t-4 border-black bg-[var(--bg-card)] px-4 pb-4 pt-3 md:hidden">
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
      <CommandPalette
        open={commandOpen}
        query={commandQuery}
        setQuery={setCommandQuery}
        inputRef={commandInputRef}
        items={commandItems}
        loading={commandLoading}
        error={commandError}
        copy={copy}
        onClose={closeCommand}
        onRun={runCommand}
        canSearchServers={Boolean(user)}
      />
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
  open,
  query,
  setQuery,
  inputRef,
  items,
  loading,
  error,
  copy,
  onClose,
  onRun,
  canSearchServers,
}: {
  open: boolean;
  query: string;
  setQuery: (value: string) => void;
  inputRef: React.RefObject<HTMLInputElement | null>;
  items: CommandItem[];
  loading: boolean;
  error: string | null;
  copy: Translations;
  onClose: () => void;
  onRun: (href: string) => void;
  canSearchServers: boolean;
}) {
  if (!open) return null;
  const pageItems = items.filter((item) => item.group === "pages");
  const shortcutItems = items.filter((item) => item.group === "commands");
  const serverItems = items.filter((item) => item.group === "servers");

  return (
    <div className="fixed inset-0 z-50 bg-black/50 px-4 py-16" role="dialog" aria-modal="true">
      <div className="mx-auto max-w-2xl border-4 border-black bg-[var(--bg-card)] shadow-[10px_10px_0_0_#000]">
        <div className="flex items-center gap-3 border-b-4 border-black bg-[var(--accent-bg)] p-3">
          <input
            ref={inputRef}
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            className={`${inputClass} min-w-0 flex-1`}
            placeholder={copy.command.searchPlaceholder}
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

function applyDashboardTheme(theme: ThemeDefinition | null) {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  const body = document.body;
  const previousKeys = (root.dataset.xlstatusDashboardThemeVars || "")
    .split(",")
    .map((key) => key.trim())
    .filter(Boolean);
  for (const key of previousKeys) {
    root.style.removeProperty(key);
    body?.style.removeProperty(key);
  }
  root.dataset.xlstatusDashboardThemeVars = "";

  const styleId = "xlstatus-dashboard-theme-style";
  document.getElementById(styleId)?.remove();

  if (!theme) return;
  const nextKeys: string[] = [];
  for (const [key, value] of Object.entries(theme.variables ?? {})) {
    if (!key.startsWith("--") || !value) continue;
    root.style.setProperty(key, value);
    body?.style.setProperty(key, value);
    nextKeys.push(key);
  }
  root.dataset.xlstatusDashboardThemeVars = nextKeys.join(",");

  if (theme.custom_css) {
    const style = document.createElement("style");
    style.id = styleId;
    style.textContent = theme.custom_css;
    document.head.appendChild(style);
  }
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
