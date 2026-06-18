"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { useState } from "react";
import { apiClient } from "@/lib/api";
import { isAdmin, useBoldTheme, useStoredUser } from "./M7Primitives";

const navigation = [
  { name: "Dashboard", href: "/dashboard" },
  { name: "Servers", href: "/servers" },
  { name: "Services", href: "/services" },
  { name: "Tasks", href: "/tasks" },
  { name: "Terminal", href: "/terminal" },
  { name: "Alerts", href: "/alerts" },
  { name: "NAT", href: "/nat" },
  { name: "DDNS", href: "/ddns", adminOnly: true },
  { name: "Settings", href: "/settings" },
  { name: "Status", href: "/status" },
];

const publicNavigation = [{ name: "Status", href: "/status" }];

export default function Navigation() {
  const pathname = usePathname();
  const user = useStoredUser();
  const [theme, setTheme] = useBoldTheme();
  const [open, setOpen] = useState(false);

  const handleLogout = async () => {
    await apiClient.logout();
    localStorage.removeItem("session_token");
    localStorage.removeItem("user");
    window.location.href = "/login";
  };

  const visibleNavigation = user
    ? navigation.filter((item) => !item.adminOnly || isAdmin(user))
    : publicNavigation;

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
            <ThemeSegment theme={theme} setTheme={setTheme} />
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
                Logout
              </button>
            ) : (
              <Link
                href="/login"
                className="border-2 border-black bg-black px-3 py-2 text-xs font-black uppercase text-white shadow-[var(--shadow-brutal-sm)]"
              >
                Login
              </Link>
            )}
          </div>

          <button
            type="button"
            onClick={() => setOpen((value) => !value)}
            className="border-2 border-black bg-[var(--bg-card)] px-3 py-2 text-sm font-black uppercase shadow-[var(--shadow-brutal-sm)] md:hidden"
            aria-label="Toggle navigation"
          >
            Menu
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
            <ThemeSegment theme={theme} setTheme={setTheme} />
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
                Logout
              </button>
            ) : (
              <Link
                href="/login"
                onClick={() => setOpen(false)}
                className="border-2 border-black bg-black px-3 py-2 text-xs font-black uppercase text-white"
              >
                Login
              </Link>
            )}
          </div>
        </div>
      ) : null}
    </nav>
  );
}

function ThemeSegment({
  theme,
  setTheme,
}: {
  theme: "light" | "dark";
  setTheme: (mode: "light" | "dark") => void;
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
          {mode}
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
