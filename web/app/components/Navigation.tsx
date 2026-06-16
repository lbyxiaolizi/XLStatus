'use client';

import Link from 'next/link';
import { usePathname } from 'next/navigation';
import { apiClient } from '@/lib/api';

const navigation = [
  { name: 'Dashboard', href: '/dashboard' },
  { name: 'Servers', href: '/servers' },
  { name: 'Services', href: '/services' },
  { name: 'Tasks', href: '/tasks' },
  { name: 'Alerts', href: '/alerts' },
  { name: 'NAT', href: '/nat' },
  { name: 'Settings', href: '/settings' },
];

export default function Navigation() {
  const pathname = usePathname();

  const handleLogout = async () => {
    await apiClient.logout();
    window.location.href = '/login';
  };

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
                {navigation.map((item) => {
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
          <div>
            <button
              onClick={handleLogout}
              className="text-gray-300 hover:bg-gray-700 hover:text-white rounded-md px-3 py-2 text-sm font-medium"
            >
              Logout
            </button>
          </div>
        </div>
      </div>
    </nav>
  );
}
