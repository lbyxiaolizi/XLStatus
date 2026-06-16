'use client';

import Navigation from '@/app/components/Navigation';

export default function SettingsPage() {
  return (
    <div>
      <Navigation />
      <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-8">
        <h1 className="text-2xl font-bold text-gray-900 mb-6">Settings</h1>

        <div className="bg-white shadow rounded-lg divide-y divide-gray-200">
          {/* General Settings */}
          <div className="p-6">
            <h2 className="text-lg font-medium text-gray-900 mb-4">General</h2>
            <div className="space-y-4">
              <div>
                <label className="block text-sm font-medium text-gray-700">
                  Site Name
                </label>
                <input
                  type="text"
                  defaultValue="XLStatus"
                  className="mt-1 block w-full rounded-md border-gray-300 shadow-sm focus:border-blue-500 focus:ring-blue-500"
                />
              </div>
              <div>
                <label className="block text-sm font-medium text-gray-700">
                  Admin Email
                </label>
                <input
                  type="email"
                  placeholder="admin@example.com"
                  className="mt-1 block w-full rounded-md border-gray-300 shadow-sm focus:border-blue-500 focus:ring-blue-500"
                />
              </div>
            </div>
          </div>

          {/* Notification Settings */}
          <div className="p-6">
            <h2 className="text-lg font-medium text-gray-900 mb-4">Notifications</h2>
            <div className="space-y-4">
              <div className="flex items-center">
                <input
                  type="checkbox"
                  id="email-notifications"
                  className="rounded border-gray-300 text-blue-600 focus:ring-blue-500"
                />
                <label htmlFor="email-notifications" className="ml-2 block text-sm text-gray-900">
                  Enable email notifications
                </label>
              </div>
              <div className="flex items-center">
                <input
                  type="checkbox"
                  id="webhook-notifications"
                  className="rounded border-gray-300 text-blue-600 focus:ring-blue-500"
                />
                <label htmlFor="webhook-notifications" className="ml-2 block text-sm text-gray-900">
                  Enable webhook notifications
                </label>
              </div>
            </div>
          </div>

          {/* User Management */}
          <div className="p-6">
            <h2 className="text-lg font-medium text-gray-900 mb-4">Users</h2>
            <button className="bg-blue-600 hover:bg-blue-700 text-white font-medium py-2 px-4 rounded">
              Manage Users
            </button>
          </div>

          {/* API Keys */}
          <div className="p-6">
            <h2 className="text-lg font-medium text-gray-900 mb-4">API Keys</h2>
            <button className="bg-blue-600 hover:bg-blue-700 text-white font-medium py-2 px-4 rounded">
              Manage API Keys
            </button>
          </div>
        </div>

        <div className="mt-6 flex justify-end">
          <button className="bg-blue-600 hover:bg-blue-700 text-white font-medium py-2 px-4 rounded">
            Save Changes
          </button>
        </div>
      </div>
    </div>
  );
}
