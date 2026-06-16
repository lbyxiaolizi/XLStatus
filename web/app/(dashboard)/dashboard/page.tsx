"use client";

import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";

interface User {
  id: string;
  username: string;
  role: string;
}

export default function DashboardPage() {
  const router = useRouter();
  const [user, setUser] = useState<User | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    // Check if user is logged in
    const sessionToken = localStorage.getItem("session_token");
    const userStr = localStorage.getItem("user");

    if (!sessionToken || !userStr) {
      router.push("/login");
      return;
    }

    setUser(JSON.parse(userStr));
    setLoading(false);
  }, [router]);

  const handleLogout = () => {
    localStorage.removeItem("session_token");
    localStorage.removeItem("user");
    router.push("/login");
  };

  if (loading) {
    return (
      <div className="min-h-screen flex items-center justify-center">
        <div className="text-gray-600">Loading...</div>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-gray-100">
      {/* Header */}
      <header className="bg-white shadow">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-4 flex justify-between items-center">
          <h1 className="text-2xl font-bold text-gray-900">XLStatus Dashboard</h1>
          <div className="flex items-center gap-4">
            <span className="text-sm text-gray-600">
              {user?.username} ({user?.role})
            </span>
            <button
              onClick={handleLogout}
              className="px-4 py-2 bg-red-600 text-white rounded hover:bg-red-700"
            >
              Logout
            </button>
          </div>
        </div>
      </header>

      {/* Main Content */}
      <main className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-8">
        <div className="grid grid-cols-1 md:grid-cols-3 gap-6 mb-8">
          {/* Stats Cards */}
          <div className="bg-white rounded-lg shadow p-6">
            <h3 className="text-lg font-semibold text-gray-700 mb-2">Servers</h3>
            <p className="text-3xl font-bold text-blue-600">0</p>
            <p className="text-sm text-gray-500 mt-1">Total servers</p>
          </div>

          <div className="bg-white rounded-lg shadow p-6">
            <h3 className="text-lg font-semibold text-gray-700 mb-2">Services</h3>
            <p className="text-3xl font-bold text-green-600">0</p>
            <p className="text-sm text-gray-500 mt-1">Active services</p>
          </div>

          <div className="bg-white rounded-lg shadow p-6">
            <h3 className="text-lg font-semibold text-gray-700 mb-2">Alerts</h3>
            <p className="text-3xl font-bold text-red-600">0</p>
            <p className="text-sm text-gray-500 mt-1">Active alerts</p>
          </div>
        </div>

        {/* Navigation */}
        <div className="bg-white rounded-lg shadow p-6">
          <h2 className="text-xl font-bold text-gray-900 mb-4">Quick Actions</h2>
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
            <a href="/servers" className="p-4 border border-gray-200 rounded hover:bg-gray-50">
              <h3 className="font-semibold text-gray-900">Servers</h3>
              <p className="text-sm text-gray-600 mt-1">Manage your servers</p>
            </a>
            <a href="/services" className="p-4 border border-gray-200 rounded hover:bg-gray-50">
              <h3 className="font-semibold text-gray-900">Services</h3>
              <p className="text-sm text-gray-600 mt-1">Monitor services</p>
            </a>
            <a href="/alerts" className="p-4 border border-gray-200 rounded hover:bg-gray-50">
              <h3 className="font-semibold text-gray-900">Alerts</h3>
              <p className="text-sm text-gray-600 mt-1">Configure alerts</p>
            </a>
            <a href="/tokens" className="p-4 border border-gray-200 rounded hover:bg-gray-50">
              <h3 className="font-semibold text-gray-900">Access Tokens</h3>
              <p className="text-sm text-gray-600 mt-1">Manage API tokens</p>
            </a>
            <a href="/users" className="p-4 border border-gray-200 rounded hover:bg-gray-50">
              <h3 className="font-semibold text-gray-900">Users</h3>
              <p className="text-sm text-gray-600 mt-1">User management</p>
            </a>
            <a href="/settings" className="p-4 border border-gray-200 rounded hover:bg-gray-50">
              <h3 className="font-semibold text-gray-900">Settings</h3>
              <p className="text-sm text-gray-600 mt-1">System settings</p>
            </a>
          </div>
        </div>

        {/* Welcome Message */}
        <div className="mt-8 bg-blue-50 border border-blue-200 rounded-lg p-6">
          <h3 className="text-lg font-semibold text-blue-900 mb-2">Welcome to XLStatus</h3>
          <p className="text-blue-800">
            Your server monitoring system is ready. Start by adding your first server or configuring services to monitor.
          </p>
          <p className="text-sm text-blue-700 mt-2">
            M1 Base Platform: Authentication, PAT, and RBAC are now functional.
          </p>
        </div>
      </main>
    </div>
  );
}
