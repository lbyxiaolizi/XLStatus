'use client';

import Link from 'next/link';
import { usePathname } from 'next/navigation';
import { useState } from 'react';
import { apiClient } from '@/lib/api';
import { isAdmin, useStoredUser } from './M7Primitives';

const navigation = [
  { name: 'Dashboard', href: '/dashboard' },
  { name: 'Servers', href: '/servers' },
  { name: 'Services', href: '/services' },
  { name: 'Tasks', href: '/tasks' },
  { name: 'Terminal', href: '/terminal' },
  { name: 'Alerts', href: '/alerts' },
  { name: 'NAT', href: '/nat' },
  { name: 'DDNS', href: '/ddns', adminOnly: true },
  { name: 'Settings', href: '/settings' },
  { name: 'Status', href: '/status' },
];

const publicNavigation = [{ name: 'Status', href: '/status' }];

export default function Navigation() {
  const pathname = usePathname();
  const user = useStoredUser();
  const [open, setOpen] = useState(false);

  const handleLogout = async () => {
    await apiClient.logout();
    localStorage.removeItem('session_token');
    localStorage.removeItem('user');
    window.location.href = '/login';
  };

  const visibleNavigation = user
    ? navigation.filter((item) => !item.adminOnly || isAdmin(user))
    : publicNavigation;

  return (
    <nav className="bg-gray-800">
      <div className="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
        <div className="flex h-16 items-center justify-between">
          <div className="flex items-center">
            <div className="flex-shrink-0">
              <h1 className="text-white text-xl font-bold">XLStatus</h1>
            </div>
            <div className="hidden md:block">
              <div className="ml-10 flex items-baseline space-x-4">
                {visibleNavigation.map((item) => {
                  const isActive = pathname === item.href;
                  return (
                    <Link
                      key={item.name}
                      href={item.href}
                      className={`${
                        isActive
                          ? 'bg-gray-900 text-white'
                          : 'text-gray-300 hover:bg-gray-700 hover:text-white'
                      } rounded-md px-3 py-2 text-sm font-medium`}
                    >
                      {item.name}
                    </Link>
                  );
                })}
              </div>
            </div>
          </div>
          <div className="hidden items-center gap-3 md:flex">
            {user ? (
              <span className="text-sm text-gray-300">
                {user.username} ({user.role})
              </span>
            ) : null}
            {user ? (
              <button
                onClick={handleLogout}
                className="text-gray-300 hover:bg-gray-700 hover:text-white rounded-md px-3 py-2 text-sm font-medium"
              >
                Logout
              </button>
            ) : (
              <Link
                href="/login"
                className="text-gray-300 hover:bg-gray-700 hover:text-white rounded-md px-3 py-2 text-sm font-medium"
              >
                Login
              </Link>
            )}
          </div>
          <button
            type="button"
            onClick={() => setOpen((value) => !value)}
            className="rounded-md px-3 py-2 text-sm font-medium text-gray-300 hover:bg-gray-700 hover:text-white md:hidden"
            aria-label="Toggle navigation"
          >
            Menu
          </button>
        </div>
      </div>
      {open ? (
        <div className="border-t border-gray-700 px-4 pb-4 pt-2 md:hidden">
          <div className="grid gap-1">
            {visibleNavigation.map((item) => {
              const active = pathname === item.href;
              return (
                <Link
                  key={item.name}
                  href={item.href}
                  onClick={() => setOpen(false)}
                  className={`rounded-md px-3 py-2 text-sm font-medium ${
                    active
                      ? 'bg-gray-900 text-white'
                      : 'text-gray-300 hover:bg-gray-700 hover:text-white'
                  }`}
                >
                  {item.name}
                </Link>
              );
            })}
            {user ? (
              <div className="px-3 py-2 text-sm text-gray-300">
                {user.username} ({user.role})
              </div>
            ) : null}
            {user ? (
              <button
                onClick={handleLogout}
                className="rounded-md px-3 py-2 text-left text-sm font-medium text-gray-300 hover:bg-gray-700 hover:text-white"
              >
                Logout
              </button>
            ) : (
              <Link
                href="/login"
                onClick={() => setOpen(false)}
                className="rounded-md px-3 py-2 text-sm font-medium text-gray-300 hover:bg-gray-700 hover:text-white"
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
